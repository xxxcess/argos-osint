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
    assert!(sent.len() < paragraph_blocks(&story()).len());
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
async fn classifier_missing_or_failing_keeps_every_uncertain_block() {
    let raw = story();
    let none = clean_article(
        &raw,
        "Geneva talks continue",
        "",
        "https://x.test/s",
        &[],
        None,
    )
    .await;
    // Ambiguous blocks (the byline) stay; only unmistakable chrome is removed
    // locally, the same with or without a working Classifier.
    assert!(none.markdown.contains("By Zoë Müller, Reuters"));
    assert!(none
        .diagnostics
        .iter()
        .any(|d| d.contains("classifier unavailable")));
    let mut failing = mock(vec!["By Zoë"]);
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
    assert_eq!(failed.markdown, none.markdown);
    assert_eq!(failed.removed, none.removed);
    assert!(failed.markdown.contains("By Zoë Müller, Reuters"));
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

#[tokio::test]
async fn nlp_mentions_named_in_the_headline_become_keep_anchors() {
    let title = "Ferry halted after harbour master review";
    let description = "Northwind ferries stopped sailing.";
    let filler = "Officials described the timetable and the operator's obligations in careful detail for readers, explaining each step of the review. ";
    let short = "The harbour master said sailings stay paused.";
    let baseline = format!(
        "{title}\n\n{}\n\n{short}\n\n{}",
        filler.repeat(3),
        filler.repeat(3)
    );
    let start = baseline.find("harbour master said").unwrap();
    let mentions = vec![
        crate::nlp::Mention {
            span: crate::nlp::Span {
                start_byte: start,
                end_byte: start + "harbour master".len(),
            },
            kind: "role".into(),
        },
        // A body-only entity is not an anchor: it is not in the headline.
        crate::nlp::Mention {
            span: crate::nlp::Span {
                start_byte: 0,
                end_byte: 0,
            },
            kind: "org".into(),
        },
    ];
    let anchors = nlp_anchors(&baseline, &mentions, title, description);
    assert!(anchors.iter().any(|a| a == "harbour master"), "{anchors:?}");
    assert!(
        anchors.iter().any(|a| a == "Northwind"),
        "description tokens anchor too"
    );
    let block = BodyChunk {
        id: 1,
        start: 0,
        end: short.chars().count(),
        text: short.into(),
    };
    assert!(
        is_ambiguous(&block, &title_anchors(title)),
        "title tokens alone miss it"
    );
    assert!(
        !is_ambiguous(&block, &anchors),
        "NLP anchor keeps the block without a model"
    );
    // End to end: the Mock removes it if asked, so survival proves it was kept
    // without a Classifier decision.
    let mut m = mock(vec!["harbour master said"]);
    let out = clean_article(
        &baseline,
        title,
        description,
        "https://x.test/s",
        &anchors,
        Some(&mut m),
    )
    .await;
    assert!(out.markdown.contains(short));
}

fn chunk(text: &str) -> Vec<BodyChunk> {
    paragraph_blocks(text)
}

const PROSE: &str = "Regulators opened an inquiry on Monday after the company disclosed that its quarterly results had been misstated, and the chief executive said the board would cooperate fully with investigators. ";

// §7: "shares fell" and substantive "related" prose stay local.
#[test]
fn cue_words_inside_story_prose_stay_local() {
    for sentence in [
        "Shares fell 12 percent in early trading as investors weighed the disclosure and analysts cut their forecasts for the year.",
        "Prosecutors said the payments were related to a consulting contract signed in 2019, which the company has denied was improper.",
        "Readers can share tips securely, the newspaper said, adding that comments from the ministry were still awaited this week.",
    ] {
        let text = format!("{PROSE}{PROSE}\n\n{sentence}\n\n{PROSE}{PROSE}");
        let blocks = chunk(&text);
        let idx = blocks.iter().position(|b| b.text.contains(&sentence[..20])).unwrap();
        assert_eq!(classify_block(&blocks, idx, &[]), BlockClass::Story, "{sentence}");
    }
}

#[test]
fn structural_chrome_is_removed_and_uncertain_blocks_reviewed() {
    let text = format!(
        "{PROSE}{PROSE}\n\n## Related stories\n\n- [One](https://x.test/1)\n- [Two](https://x.test/2)\n- [Three](https://x.test/3)\n\nShare\n\nSubscribe to our newsletter for daily updates\n\n© 2026 Example News. All rights reserved.\n\nBy Staff Writer\n\n{PROSE}{PROSE}"
    );
    let blocks = chunk(&text);
    let class_of = |needle: &str| {
        let i = blocks.iter().position(|b| b.text.contains(needle)).unwrap();
        classify_block(&blocks, i, &[])
    };
    assert_eq!(class_of("Related stories"), BlockClass::Chrome);
    assert_eq!(class_of("[Two]"), BlockClass::Chrome);
    assert_eq!(class_of("Share"), BlockClass::Chrome);
    assert_eq!(class_of("Subscribe to"), BlockClass::Chrome);
    assert_eq!(class_of("© 2026"), BlockClass::Chrome);
    assert_eq!(class_of("By Staff Writer"), BlockClass::Ambiguous);
    assert_eq!(class_of("Regulators opened"), BlockClass::Story);
}

#[tokio::test]
async fn ordinary_story_sends_no_classifier_batch() {
    let text = format!(
        "{PROSE}{PROSE}\n\nShares fell sharply after the announcement, the exchange said in a short statement.\n\n\"We did not mislead anyone,\" the chief executive said.\n\nCorrection: an earlier version misstated the date of the inquiry.\n\n{PROSE}{PROSE}"
    );
    let mut m = mock(vec![]);
    let a = clean_article(
        &text,
        "Regulators open inquiry",
        "",
        "https://x.test/s",
        &[],
        Some(&mut m),
    )
    .await;
    assert!(m.batches.is_empty(), "{:?}", m.batches);
    for kept in ["Shares fell", "We did not mislead", "Correction:"] {
        assert!(a.markdown.contains(kept), "{kept}");
    }
}

#[test]
fn soft_wraps_join_within_paragraphs_preserving_offsets() {
    let raw = "The minister said the talks\nwould continue next week, and\nthe envoy agreed.\n\n- item one\n- item two\n\n# Heading\nFirst line ends here.\nSecond sentence starts a new line.\nLine with hard break  \nnext line\n\n“Zoë said\nthis.”";
    let (out, joined) = unwrap_soft_wraps(raw);
    assert_eq!(out.len(), raw.len(), "byte length preserved");
    assert!(
        out.contains("The minister said the talks would continue next week, and the envoy agreed.")
    );
    assert!(out.contains("- item one\n- item two"), "lists untouched");
    assert!(out.contains("# Heading\nFirst line"), "headings untouched");
    assert!(
        out.contains("ends here.\nSecond sentence"),
        "sentence-final line kept"
    );
    assert!(
        out.contains("hard break  \nnext"),
        "markdown hard break kept"
    );
    assert!(out.contains("“Zoë said this.”"), "unicode joined");
    assert!(out.contains("\n\n"), "paragraph breaks kept");
    assert_eq!(joined, 3);
    // Every non-newline byte is at the same offset.
    for ((a, b), _) in raw.bytes().zip(out.bytes()).zip(0..) {
        if a != b {
            assert!(a == b'\n' || a == b'\r');
            assert_eq!(b, b' ');
        }
    }
}
