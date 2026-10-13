use super::*;

struct Mock {
    remove: Vec<&'static str>,
    batches: Vec<Vec<usize>>,
    fail: bool,
}

impl BlockDecider for Mock {
    fn decide<'a>(
        &'a mut self,
        batch: &'a [BodyChunk],
        _context: &'a [Value],
    ) -> Pin<Box<dyn Future<Output = Result<Vec<IrrelevantRange>>> + Send + 'a>> {
        Box::pin(async move {
            self.batches.push(batch.iter().map(|b| b.id).collect());
            anyhow::ensure!(!self.fail, "classifier offline");
            Ok(batch
                .iter()
                .filter(|b| self.remove.iter().any(|m| b.text.contains(m)))
                .map(|b| IrrelevantRange {
                    start: b.start,
                    end: b.end,
                    reason: "chrome".into(),
                })
                .collect())
        })
    }
}

fn mock(remove: Vec<&'static str>) -> Mock {
    Mock {
        remove,
        batches: Vec::new(),
        fail: false,
    }
}

fn story() -> String {
    let para = "Officials in Geneva said on Tuesday that the negotiations would continue, although the delegation denied reports that a deal had been reached and declined to give a timeline for further talks. ";
    format!(
        "Geneva talks continue\n\nBy Zoë Müller, Reuters\n\n{}\n\n\"We have not agreed to anything,\" the envoy said — a quote that must survive.\n\nCorrection: an earlier version misstated the date as Monday.\n\nAdvertisement — Subscribe to our newsletter\n\n{}\n\nRelated stories [a](https://x.test/a) [b](https://x.test/b) [c](https://x.test/c)\n\n{}",
        para.repeat(3),
        para.repeat(3),
        para.repeat(3)
    )
}

#[tokio::test]
async fn preserves_byline_quote_negation_correction_and_unicode_verbatim() {
    let mut m = mock(vec!["Advertisement", "Related stories"]);
    let a = clean_article(
        &story(),
        "Geneva talks continue",
        "",
        "https://x.test/s",
        &title_anchors("Geneva talks continue"),
        Some(&mut m),
    )
    .await;
    for kept in [
        "By Zoë Müller",
        "\"We have not agreed to anything,\"",
        "denied reports",
        "Correction: an earlier version",
        "declined to give",
    ] {
        assert!(a.markdown.contains(kept), "lost {kept}");
    }
    assert!(!a.markdown.contains("Subscribe to our newsletter"));
    assert!(!a.markdown.contains("Related stories"));
    assert!(!a.removed.is_empty());
    assert_eq!(
        a.content_hash,
        super::super::validate::content_hash(&a.markdown)
    );
    // Clear prose is never sent to the model.
    let sent: Vec<usize> = m.batches.concat();
    assert!(sent.len() < chunk_article_body(&story()).len());
}

#[tokio::test]
async fn span_maps_point_to_identical_baseline_text() {
    let mut m = mock(vec!["Advertisement"]);
    let raw = story();
    let a = clean_article(
        &raw,
        "Geneva talks continue",
        "",
        "https://x.test/s",
        &[],
        Some(&mut m),
    )
    .await;
    let baseline =
        validate_article_body(&raw, "Geneva talks continue", "https://x.test/s").cleaned_markdown;
    for span in &a.spans {
        let len = span.baseline_end - span.baseline_start;
        assert_eq!(
            &baseline[span.baseline_start..span.baseline_end],
            &a.markdown[span.clean_start..span.clean_start + len]
        );
    }
}

#[tokio::test]
async fn classifier_missing_or_failing_keeps_every_block() {
    let raw = story();
    let base =
        validate_article_body(&raw, "Geneva talks continue", "https://x.test/s").cleaned_markdown;
    let none = clean_article(
        &raw,
        "Geneva talks continue",
        "",
        "https://x.test/s",
        &[],
        None,
    )
    .await;
    assert_eq!(none.markdown, base);
    assert!(none
        .diagnostics
        .iter()
        .any(|d| d.contains("classifier unavailable")));
    let mut failing = mock(vec!["Advertisement"]);
    failing.fail = true;
    let failed = clean_article(
        &raw,
        "Geneva talks continue",
        "",
        "https://x.test/s",
        &[],
        Some(&mut failing),
    )
    .await;
    assert_eq!(failed.markdown, base);
    assert!(failed.removed.is_empty());
}

#[tokio::test]
async fn ambiguous_blocks_go_in_sequential_batches_of_at_most_twelve() {
    let mut raw = String::from("Title here\n\n");
    for i in 0..30 {
        raw.push_str(&format!("Share item {i} [x](https://x.test/{i})\n\n"));
        raw.push_str(&"Ministers said the plan would proceed after the vote, but critics disputed the figures in detail. ".repeat(3));
        raw.push_str("\n\n");
    }
    let mut m = mock(vec![]);
    let a = clean_article(
        &raw,
        "Title here",
        "",
        "https://x.test/s",
        &[],
        Some(&mut m),
    )
    .await;
    assert!(m.batches.len() >= 3);
    assert!(m.batches.iter().all(|b| b.len() <= BATCH));
    let flat: Vec<usize> = m.batches.concat();
    assert!(
        flat.windows(2).all(|w| w[0] < w[1]),
        "batches must be sequential"
    );
    assert_eq!(a.batches, m.batches.len());
}

#[tokio::test]
async fn removals_that_degrade_coverage_are_rejected() {
    let raw = story();
    let mut greedy = Mock {
        remove: vec![""],
        batches: Vec::new(),
        fail: false,
    };
    let base = validate_article_body(&raw, "Geneva talks continue", "https://x.test/s");
    let a = clean_article(
        &raw,
        "Geneva talks continue",
        "",
        "https://x.test/s",
        &[],
        Some(&mut greedy),
    )
    .await;
    assert!(rank(&base.quality) <= rank_str(&a.quality));
}

fn rank_str(q: &str) -> u8 {
    ["unavailable", "uncertain", "partial", "complete"]
        .iter()
        .position(|x| *x == q)
        .unwrap_or(0) as u8
}

#[tokio::test]
async fn incomplete_or_missing_story_fails_stage_one() {
    let a = clean_article(
        &story(),
        "Geneva talks continue",
        "",
        "https://x.test/s",
        &[],
        None,
    )
    .await;
    assert!(stage_one_admissible(&a, false).is_err());
    let empty = clean_article("", "t", "", "https://x.test/s", &[], None).await;
    assert!(stage_one_admissible(&empty, true).is_err());
}
