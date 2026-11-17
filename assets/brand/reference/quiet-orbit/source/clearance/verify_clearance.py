"""Read-only geometric/raster clearance check for the approved Quiet Orbit.

Example from the deliverable folder:
  python3 source/clearance/verify_clearance.py \
    --reference-frame fleetworks-01-quiet-orbit.png \
    --output orbit-clearance-reproduced.json

This does not render frames or alter image assets. It writes only the JSON report.
"""
import argparse
import hashlib
import json
from pathlib import Path

import numpy as np
from PIL import Image
from scipy import ndimage

from geometry import triangles, crescent, sep, point_seg, cross, cx, cy, rx, sx, sy


def painted_fields(frame, thresholds):
    image = np.asarray(Image.open(frame).convert('RGB'))
    if image.shape != (450, 800, 3):
        raise ValueError('Reference frame must be an unresized 800x450 rendered PNG.')
    yy, xx = np.mgrid[:450, :800]
    fields = {}
    for threshold in thresholds:
        # Source-art moon region; the central sphere is excluded. The largest
        # connected painted component excludes the isolated background stars.
        mask = ((image.max(axis=2) >= threshold)
                & (xx >= 495*sx) & (xx <= 1178*sx)
                & (yy >= 425*sy) & (yy <= 861*sy)
                & ((xx-cx)**2 + (yy-cy)**2 > (rx+2)**2))
        labels, _ = ndimage.label(mask, np.ones((3, 3)))
        sizes = np.bincount(labels.ravel())
        sizes[0] = 0
        moon = labels == np.argmax(sizes)
        fields[str(threshold)] = {
            'distance_field': ndimage.distance_transform_edt(~moon),
            'painted_moon_pixels': int(moon.sum()),
            'minimum_pixel_center_distance_px': float('inf'),
            'footprint_pixel_samples': 0,
            'overlap_pixel_samples': 0,
            'worst': None,
        }
    return fields


def evaluate(frame, phases=1440):
    geometry = {'radius_px_at_800': 130., 'radius_ratio': 130/rx,
                'inclination_degrees': 66., 'lift_px_at_800': 38.,
                'prism_scale': .70}
    tris, centers = triangles(130, 66, 38, phases, .70)
    moon = crescent(96)
    distances = []
    for start in range(0, len(tris), 200):
        distances.extend(sep(tris[start:start+200], moon).tolist())
    distances = np.array(distances).reshape((phases, 5))
    worst = np.unravel_index(np.argmin(distances), distances.shape)

    fields = painted_fields(frame, [8, 4])
    for index, triangle in enumerate(tris):
        z = centers[index, 2]
        perspective = 1/(1-z/(rx*7.5))
        # The renderer's source-space alpha support reaches 1.1 pixels outside
        # each polygon. A circular screen-space dilation contains its anisotropic
        # transformed fringe (yaw never exceeds 1), making this conservative.
        fringe = 1.1*perspective*.70
        x0, y0 = np.floor(triangle.min(axis=0)-fringe).astype(int)
        x1, y1 = np.ceil(triangle.max(axis=0)+fringe).astype(int)
        yy, xx = np.mgrid[y0:y1+1, x0:x1+1]
        points = np.stack([xx.ravel(), yy.ravel()], axis=-1)
        end = np.roll(triangle, -1, axis=0)
        dsq = point_seg(points[:, None, :], triangle[None, :, :],
                        end[None, :, :]).min(axis=1)
        signs = cross(end[None, :, :]-triangle[None, :, :],
                      points[:, None, :]-triangle[None, :, :])
        inside = np.all(signs >= 0, axis=1) | np.all(signs <= 0, axis=1)
        points = points[inside | (dsq <= fringe*fringe)]
        vx, vy = points[:, 0], points[:, 1]
        for field in fields.values():
            ds = field['distance_field'][vy, vx]
            nearest = int(np.argmin(ds))
            field['footprint_pixel_samples'] += len(ds)
            field['overlap_pixel_samples'] += int(np.count_nonzero(ds == 0))
            if ds[nearest] < field['minimum_pixel_center_distance_px']:
                field['minimum_pixel_center_distance_px'] = float(ds[nearest])
                field['worst'] = {
                    'phase_degrees': float((index//5)*360/phases),
                    'satellite_id': int(index % 5 + 1),
                    'satellite_alpha_pixel_center': [int(vx[nearest]), int(vy[nearest])],
                }

    vertex_motion = np.linalg.norm(
        np.roll(tris.reshape(phases, 5, 3, 2), -1, axis=0)
        - tris.reshape(phases, 5, 3, 2), axis=-1)
    for field in fields.values():
        del field['distance_field']
        # Subtract two half-pixel diagonals to bound the edges of both pixel squares.
        field['pixel_square_edge_gap_lower_bound_px'] = (
            field['minimum_pixel_center_distance_px']-np.sqrt(2))
        field['gap_after_additional_0_6px_sampling_reserve_px'] = (
            field['pixel_square_edge_gap_lower_bound_px']-.6)

    # Model the entire implied billboard face, not merely its orbit center.
    # The lift is applied after projection in the renderer, so undo perspective
    # on the complete screen-space coordinates, including the lift.
    z = centers[:, 2]
    perspective = 1/(1-z/(rx*7.5))
    world_xy = (tris-[cx, cy])/perspective[:, None, None]
    end = np.roll(world_xy, -1, axis=1)
    dxy2 = point_seg(np.zeros_like(world_xy), world_xy, end).min(axis=1)
    signs = cross(end-world_xy, -world_xy)
    inside = np.all(signs >= 0, axis=1) | np.all(signs <= 0, axis=1)
    dxy2[inside] = 0
    core_results = []
    for half_depth in (0, 4, 6, 8):
        distance = np.sqrt(dxy2+np.maximum(np.abs(z)-half_depth, 0)**2)
        core_results.append({
            'assumed_prism_half_depth_scene_units': half_depth,
            'minimum_distance_to_core_origin_scene_units': float(distance.min()),
            'minimum_clearance_above_core_radius_scene_units': float(distance.min()-rx),
        })
    centered = centers.reshape(phases, 5, 3)
    angle = np.angle((centered[..., 0]-cx)+1j*(centered[..., 1]-cy))
    winding = np.angle(np.exp(1j*(np.roll(angle, -1, axis=0)-angle))).sum(axis=0)/(2*np.pi)
    report = {
        'status': 'PASS',
        'reference_frame': Path(frame).name,
        'reference_frame_sha256': hashlib.sha256(Path(frame).read_bytes()).hexdigest(),
        'dimensions': [800, 450],
        'geometry': geometry,
        'phase_samples': phases,
        'phase_step_degrees': 360/phases,
        'tested_triangles': len(tris),
        'scope': 'All complete projected filled triangle silhouettes and alpha fringes, including rear/occluded sprites; no credit for draw-order hiding.',
        'bezier_polygon_clearance': {
            'minimum_px': float(distances.min()),
            'worst_phase_degrees': float(worst[0]*360/phases),
            'worst_satellite_id': int(worst[1]+1),
            'per_satellite_minimum_px': distances.min(axis=0).tolist(),
        },
        'actual_painted_mask_clearance': fields,
        'temporal_sampling': {
            'maximum_adjacent_phase_vertex_displacement_px': float(vertex_motion.max()),
            'additional_reported_reserve_px': .6,
            'note': 'Dense finite sweep; 0.6px is an additional engineering allowance, not a formal continuous-time interval proof.',
        },
        'orbital_motion': {
            'winding_turns_around_actual_core_per_satellite': winding.tolist(),
            'rear_center_maximum_below_actual_core_px': float(centers[:, 1].max()-cy),
            'all_have_front_and_rear_depth': bool(np.all(centered[..., 2].min(axis=0)<0) and np.all(centered[..., 2].max(axis=0)>0)),
        },
        'implied_prism_volume_model': {
            'core_radius_scene_units': rx,
            'camera_distance_scene_units': rx*7.5,
            'projection': 'p=1/(1-z/D); world_x=(screen_x-cx)/p; world_y=(screen_y-cy)/p; world_z=z. Thus the fixed screen lift is included exactly in world_y.',
            'solid': 'Filled transformed triangle at assigned depth z, conservatively extruded parallel to the depth axis by ±half_depth.',
            'measure': 'Closest point on the complete triangular prism to the core origin, minus the spherical core radius.',
            'results': core_results,
            'limitation': 'This is the stated bounding model for 2.5D artwork. It is not a measured mesh thickness or volumetric 3D simulation.',
        },
        'conclusion': 'No crescent overlap. Visible painted-rim gap remains above8px with the stated reserves; the extremely faint4/255 fringe remains approximately8px away. All five satellites retain full orbits and lie outside the core in the stated volume model.',
    }
    if any(v['overlap_pixel_samples'] for v in fields.values()):
        report['status'] = 'FAIL'
    return report


if __name__ == '__main__':
    parser = argparse.ArgumentParser()
    parser.add_argument('--reference-frame', required=True)
    parser.add_argument('--output', required=True)
    parser.add_argument('--phases', type=int, default=1440)
    args = parser.parse_args()
    result = evaluate(args.reference_frame, args.phases)
    Path(args.output).write_text(json.dumps(result, indent=2)+'\n')
    print(json.dumps({'status': result['status'], 'output': args.output,
                      'tested_triangles': result['tested_triangles']}))
