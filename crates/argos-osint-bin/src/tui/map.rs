//! Braille world map for the Atlas history view.
//!
//! The drawing follows ttymap's map core: Web Mercator, one terminal cell as a
//! 2×4 Braille pixel, and country-name labels that refuse to overlap. Fully
//! zoomed out, land in the selected history run is colored by temperature.
//! Only those highlighted countries are named.

use std::f64::consts::PI;

use ratatui::layout::Rect;
use ratatui::style::{Color, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph, Widget};

use super::app::App;
use super::land;
use super::theme::{self, BG};
use argos_osint_core::atlas::RunStats;

const MAX_LAT: f64 = 85.051_128_78;
const LAND: Color = Color::Rgb(86, 128, 114);
const WATER: Color = Color::Rgb(12, 32, 48);
const LABEL: Color = Color::Rgb(236, 246, 240);

/// Bit layout matches ttymap's braille buffer: 2 pixels wide, 4 tall.
const BRAILLE_MAP: [[u8; 2]; 4] = [[0x01, 0x08], [0x02, 0x10], [0x04, 0x20], [0x40, 0x80]];

/// Glyph and color for one land sample. Heat is drawn only at world zoom.
pub fn land_style(zoom: u8, code: &str, heat: &[(&str, f64)]) -> (char, Color) {
    if zoom == 0 {
        if let Some((_, temp)) = heat.iter().find(|(country, _)| *country == code) {
            return ('\u{28FF}', heat_color(*temp));
        }
    }
    ('\u{28FF}', LAND)
}

/// Country, tier, and temperature from one saved run. Unscored rows are left out.
pub fn scored_heat(stats: Option<&RunStats>) -> Vec<(&str, u8, f64)> {
    let Some(stats) = stats else {
        return Vec::new();
    };
    stats
        .origins
        .iter()
        .filter(|row| row.tier > 0 && row.temperature > 0.0)
        .map(|row| (row.country.as_str(), row.tier, row.temperature))
        .collect()
}

fn heat_color(temp: f64) -> Color {
    let t = ((temp - 0.10) / 0.90).clamp(0.0, 1.0);
    let r = (90.0 + 150.0 * t) as u8;
    let g = (48.0 + 90.0 * (1.0 - t)) as u8;
    let b = (36.0 * (1.0 - t)) as u8;
    Color::Rgb(r, g, b)
}

fn mercator_y(lat: f64) -> f64 {
    let lat = lat.clamp(-MAX_LAT, MAX_LAT).to_radians();
    (1.0 - (lat.tan() + 1.0 / lat.cos()).ln() / PI) / 2.0
}

/// Pixels across the whole world. Zoom 0 fits 360° into the canvas width.
fn world_scale(zoom: u8, width: usize) -> f64 {
    width as f64 * f64::from(1_u16 << zoom.min(4))
}

fn project(
    lon: f64,
    lat: f64,
    center_lon: f64,
    center_lat: f64,
    scale: f64,
    width: f64,
    height: f64,
) -> (f64, f64) {
    let mut dx = (lon - center_lon) / 360.0;
    if dx > 0.5 {
        dx -= 1.0;
    } else if dx < -0.5 {
        dx += 1.0;
    }
    let dy = mercator_y(lat) - mercator_y(center_lat);
    (width / 2.0 + dx * scale, height / 2.0 + dy * scale)
}

fn contains(ring: &[(i16, i16)], x: f64, y: f64) -> bool {
    let mut inside = false;
    let n = ring.len();
    if n < 3 {
        return false;
    }
    let mut j = n - 1;
    for i in 0..n {
        let (xi, yi) = (f64::from(ring[i].0), f64::from(ring[i].1));
        let (xj, yj) = (f64::from(ring[j].0), f64::from(ring[j].1));
        if ((yi > y) != (yj > y)) && (x < (xj - xi) * (y - yi) / (yj - yi) + xi) {
            inside = !inside;
        }
        j = i;
    }
    inside
}

/// Country covering this latitude and longitude, if the point falls on land.
pub fn country_at(lat: f64, lon: f64) -> Option<&'static str> {
    let x = lon * 10.0;
    let y = lat * 10.0;
    for part in land::PARTS.iter().rev() {
        let Some(exterior) = part.rings.first() else {
            continue;
        };
        if contains(exterior, x, y) && part.rings[1..].iter().all(|hole| !contains(hole, x, y)) {
            return land::CODES.get(part.code as usize).copied();
        }
    }
    None
}

fn fill(grid: &mut [u8], width: i32, height: i32, poly: &[(i32, i32)], id: u8) {
    if poly.len() < 3 || width <= 0 || height <= 0 {
        return;
    }
    let min_y = poly.iter().map(|p| p.1).min().unwrap_or(0).max(0);
    let max_y = poly.iter().map(|p| p.1).max().unwrap_or(-1).min(height - 1);
    if min_y > max_y {
        return;
    }
    for y in min_y..=max_y {
        let mut nodes = Vec::new();
        let mut prev = poly.len() - 1;
        for i in 0..poly.len() {
            let (xi, yi) = poly[i];
            let (xj, yj) = poly[prev];
            if (yi < y && yj >= y) || (yj < y && yi >= y) {
                let x = f64::from(xi) + f64::from(y - yi) / f64::from(yj - yi) * f64::from(xj - xi);
                nodes.push(x);
            }
            prev = i;
        }
        nodes.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
        let mut k = 0;
        while k + 1 < nodes.len() {
            let x0 = (nodes[k].ceil() as i32).max(0);
            let x1 = (nodes[k + 1].floor() as i32).min(width - 1);
            if x0 <= x1 {
                let row = y as usize * width as usize;
                for x in x0..=x1 {
                    grid[row + x as usize] = id;
                }
            }
            k += 2;
        }
    }
}

fn ring_pixels(
    ring: &[(i16, i16)],
    center_lon: f64,
    center_lat: f64,
    scale: f64,
    width: f64,
    height: f64,
) -> Vec<(i32, i32)> {
    let mut out = Vec::with_capacity(ring.len());
    for &(lon, lat) in ring {
        let (x, y) = project(
            f64::from(lon) / 10.0,
            f64::from(lat) / 10.0,
            center_lon,
            center_lat,
            scale,
            width,
            height,
        );
        let point = (x.round() as i32, y.round() as i32);
        if out.last() != Some(&point) {
            out.push(point);
        }
    }
    out
}

fn on_screen(poly: &[(i32, i32)], width: i32, height: i32) -> bool {
    let min_x = poly.iter().map(|p| p.0).min().unwrap_or(0);
    let max_x = poly.iter().map(|p| p.0).max().unwrap_or(-1);
    let min_y = poly.iter().map(|p| p.1).min().unwrap_or(0);
    let max_y = poly.iter().map(|p| p.1).max().unwrap_or(-1);
    max_x >= 0 && min_x < width && max_y >= 0 && min_y < height
}

struct Placed {
    x: i32,
    y: i32,
    w: i32,
    h: i32,
}

fn label_fits(placed: &[Placed], x: i32, y: i32, w: i32, h: i32) -> bool {
    placed.iter().all(|box_| {
        x >= box_.x + box_.w || box_.x >= x + w || y >= box_.y + box_.h || box_.y >= y + h
    })
}

fn land_index(country: &str) -> Option<usize> {
    let code = canonical(country);
    land::CODES.iter().position(|known| *known == code)
}

fn canonical(country: &str) -> String {
    match country.trim().to_ascii_lowercase().as_str() {
        "uk" => "gb".to_string(),
        other => other.to_string(),
    }
}

/// Countries too small to survive the coastline simplify step. Coordinates are
/// degrees times 10, so a label can still be placed when there is no polygon.
const PINS: &[(&str, i16, i16)] = &[
    ("sg", 1038, 14),
    ("hk", 1142, 223),
    ("mo", 1135, 222),
    ("bh", 506, 260),
    ("mt", 145, 359),
    ("mv", 735, 32),
];

fn pin(country: &str) -> Option<(i16, i16)> {
    let code = canonical(country);
    PINS.iter()
        .find(|(known, _, _)| *known == code)
        .map(|(_, lon, lat)| (*lon, *lat))
}

fn country_pixel(grid: &[u8], width: usize, height: usize, id: u8) -> Option<(i32, i32)> {
    let mut count = 0_i64;
    let mut sum_x = 0_i64;
    let mut sum_y = 0_i64;
    for y in 0..height {
        let row = y * width;
        for x in 0..width {
            if grid[row + x] == id {
                count += 1;
                sum_x += x as i64;
                sum_y += y as i64;
            }
        }
    }
    (count > 0).then_some(((sum_x / count) as i32, (sum_y / count) as i32))
}

/// Reserve a cell for `name`. Nearby rows are tried before overlapping a name
/// that is already on the map, so a crowded region still keeps every label.
fn claim_label(
    placed: &mut Vec<Placed>,
    col: i32,
    row: i32,
    width: i32,
    cols: i32,
    rows: i32,
) -> Option<(i32, i32)> {
    if width <= 0 || width > cols || rows <= 0 {
        return None;
    }
    let col = col.clamp(0, cols - width);
    let free = |placed: &[Placed], row: i32| {
        (0..rows).contains(&row) && label_fits(placed, col, row, width, 1)
    };
    let mut chosen = None;
    if free(placed, row) {
        chosen = Some(row);
    } else {
        for dist in 1..rows {
            if free(placed, row - dist) {
                chosen = Some(row - dist);
                break;
            }
            if free(placed, row + dist) {
                chosen = Some(row + dist);
                break;
            }
        }
    }
    let row = chosen.unwrap_or_else(|| row.clamp(0, rows - 1));
    placed.push(Placed {
        x: col,
        y: row,
        w: width,
        h: 1,
    });
    Some((col, row))
}

pub fn draw_world_map(frame: &mut ratatui::Frame, app: &App, area: Rect) {
    let zoom = 0_u8;
    let stats = app
        .atlas_runs
        .get(app.atlas_run_sel)
        .and_then(|run| serde_json::from_str::<RunStats>(&run.stats_json).ok());
    let heat = scored_heat(stats.as_ref());
    let title = if zoom == 0 {
        format!(" world · {} hot ", heat.len())
    } else {
        " world ".to_string()
    };
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(theme::BORDER).bg(BG))
        .title(title)
        .title_style(theme::dim())
        .style(theme::text());
    frame.render_widget(block, area);
    let inner = Rect {
        x: area.x.saturating_add(1),
        y: area.y.saturating_add(1),
        width: area.width.saturating_sub(2),
        height: area.height.saturating_sub(2),
    };
    if inner.width < 2 || inner.height < 1 {
        return;
    }
    let cols = inner.width as usize;
    let rows = inner.height as usize;
    let width = cols * 2;
    let height = rows * 4;
    let scale = world_scale(zoom, width);
    let mut temps = vec![None; land::CODES.len()];
    for (country, _, temp) in &heat {
        if let Some(index) = land_index(country) {
            temps[index] = Some(*temp);
        }
    }
    let mut grid = vec![0_u8; width * height];
    let width_i = width as i32;
    let height_i = height as i32;
    let width_f = width as f64;
    let height_f = height as f64;
    for part in land::PARTS {
        let Some(exterior) = part.rings.first() else {
            continue;
        };
        let outer = ring_pixels(exterior, 0.0, 15.0, scale, width_f, height_f);
        if outer.len() >= 3 && on_screen(&outer, width_i, height_i) {
            fill(
                &mut grid,
                width_i,
                height_i,
                &outer,
                part.code.saturating_add(1),
            );
        }
        for hole in &part.rings[1..] {
            let inner_ring = ring_pixels(hole, 0.0, 15.0, scale, width_f, height_f);
            if inner_ring.len() >= 3 && on_screen(&inner_ring, width_i, height_i) {
                fill(&mut grid, width_i, height_i, &inner_ring, 0);
            }
        }
    }

    let mut glyphs = vec![' '; cols * rows];
    let mut colors = vec![WATER; cols * rows];
    for row in 0..rows {
        for col in 0..cols {
            let mut mask = 0_u8;
            let mut counts = [0_u8; 8];
            let mut ids = [0_u8; 8];
            let mut kinds = 0_usize;
            for sy in 0..4 {
                for sx in 0..2 {
                    let id = grid[(row * 4 + sy) * width + col * 2 + sx];
                    if id == 0 {
                        continue;
                    }
                    mask |= BRAILLE_MAP[sy][sx];
                    if let Some(slot) = ids.iter().position(|seen| *seen == id) {
                        counts[slot] = counts[slot].saturating_add(1);
                    } else if kinds < 8 {
                        ids[kinds] = id;
                        counts[kinds] = 1;
                        kinds += 1;
                    }
                }
            }
            let index = col + cols * row;
            if mask == 0 {
                continue;
            }
            let (best, _) = counts[..kinds]
                .iter()
                .zip(ids[..kinds].iter())
                .max_by_key(|(count, _)| *count)
                .map(|(count, id)| (*id, *count))
                .unwrap_or((0, 0));
            let code_index = best.saturating_sub(1) as usize;
            let color = temps
                .get(code_index)
                .and_then(|temp| temp.filter(|_| zoom == 0).map(heat_color))
                .unwrap_or(LAND);
            glyphs[index] = char::from_u32(0x2800 + u32::from(mask)).unwrap_or(' ');
            colors[index] = color;
        }
    }

    let mut placed = Vec::new();
    let mut ranked = heat;
    ranked.sort_by(|left, right| {
        right
            .2
            .partial_cmp(&left.2)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    for (country, tier, _) in ranked {
        let index = land_index(country);
        let id = index.map(|index| (index as u8).saturating_add(1));
        let Some((px, py)) = id
            .and_then(|id| country_pixel(&grid, width, height, id))
            .map(|(x, y)| (f64::from(x), f64::from(y)))
            .or_else(|| {
                let (lon, lat) = index
                    .and_then(|index| land::LABELS.get(index).copied())
                    .filter(|(lon, lat)| !(*lon == 0 && *lat == 0))
                    .or_else(|| pin(country))?;
                Some(project(
                    f64::from(lon) / 10.0,
                    f64::from(lat) / 10.0,
                    0.0,
                    15.0,
                    scale,
                    width_f,
                    height_f,
                ))
            })
        else {
            continue;
        };
        let code = index
            .map(|index| land::CODES[index].to_string())
            .unwrap_or_else(|| canonical(country));
        let code_label = code.to_ascii_uppercase();
        let name = argos_osint_core::atlas::country_name(&code).unwrap_or(code.as_str());
        let label = if tier <= 2 { name } else { code_label.as_str() };
        let w = label.chars().count() as i32;
        let Some((col, row)) = claim_label(
            &mut placed,
            (px / 2.0).round() as i32 - w / 2,
            (py / 4.0).round() as i32,
            w,
            cols as i32,
            rows as i32,
        ) else {
            continue;
        };
        for (offset, ch) in label.chars().enumerate() {
            let at = (col as usize + offset) + cols * row as usize;
            glyphs[at] = ch;
            colors[at] = LABEL;
        }
    }

    let mut lines = Vec::with_capacity(rows);
    for row in 0..rows {
        let mut spans = Vec::with_capacity(cols);
        for col in 0..cols {
            let index = col + cols * row;
            spans.push(Span::styled(
                glyphs[index].to_string(),
                Style::default().fg(colors[index]).bg(WATER),
            ));
        }
        lines.push(Line::from(spans));
    }
    Paragraph::new(lines).render(inner, frame.buffer_mut());
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn centroids_land_on_their_country() {
        assert_eq!(country_at(39.0, -98.0), Some("us"));
        assert_eq!(country_at(35.0, 104.0), Some("cn"));
        assert_eq!(country_at(51.5, -0.1), Some("gb"));
        assert_eq!(country_at(40.4, -3.7), Some("es"));
        assert_eq!(country_at(0.0, -30.0), None);
    }

    #[test]
    fn braille_bits_match_the_ttymap_layout() {
        assert_eq!(BRAILLE_MAP[0], [0x01, 0x08]);
        assert_eq!(BRAILLE_MAP[1], [0x02, 0x10]);
        assert_eq!(BRAILLE_MAP[2], [0x04, 0x20]);
        assert_eq!(BRAILLE_MAP[3], [0x40, 0x80]);
    }

    #[test]
    fn heat_keeps_scored_countries_only() {
        let stats = RunStats {
            counts: Default::default(),
            origins: vec![
                argos_osint_core::atlas::OriginStat {
                    country: "us".into(),
                    tier: 1,
                    temperature: 1.0,
                    volume: 4,
                    articles: 4,
                },
                argos_osint_core::atlas::OriginStat {
                    country: "cn".into(),
                    tier: 0,
                    temperature: 0.0,
                    volume: 1,
                    articles: 1,
                },
            ],
            scored: true,
        };
        let heat = scored_heat(Some(&stats));
        assert_eq!(heat, vec![("us", 1, 1.0)]);
    }

    #[test]
    fn heat_is_only_drawn_when_fully_zoomed_out() {
        let heat = [("us", 1.0), ("es", 0.2)];
        let (hot_glyph, hot) = land_style(0, "us", &heat);
        let (warm_glyph, warm) = land_style(0, "es", &heat);
        let (zoomed_glyph, _) = land_style(1, "us", &heat);
        assert_eq!(hot_glyph, '\u{28FF}');
        assert_eq!(warm_glyph, '\u{28FF}');
        assert_eq!(zoomed_glyph, '\u{28FF}');
        let Color::Rgb(hot_r, _, _) = hot else {
            panic!("heat uses rgb");
        };
        let Color::Rgb(warm_r, _, _) = warm else {
            panic!("heat uses rgb");
        };
        assert!(hot_r > warm_r);
        let (_, cold) = land_style(1, "us", &heat);
        let Color::Rgb(cold_r, _, _) = cold else {
            panic!("land uses rgb");
        };
        assert!(hot_r > cold_r);
    }
}
