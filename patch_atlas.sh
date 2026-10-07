sed -i '' -e '870,921c\
pub fn apply_peer_support(\
    claims: &mut [KeptClaim],\
    articles: &[AtlasArticleRow],\
    classifier_peers: &[PeerMatch],\
) {\
    for (index, claim) in claims.iter_mut().enumerate() {\
        if claim.context {\
            continue;\
        }\
        let peers: Vec<&AtlasArticleRow> = classifier_peers\
            .iter()\
            .find(|item| item.claim_index == index)\
            .map(|item| {\
                item.article_ids\
                    .iter()\
                    .filter_map(|id| {\
                        articles\
                            .iter()\
                            .find(|article| &article.id == id && article.id != claim.article_id)\
                    })\
                    .take(RELATED_LIMIT)\
                    .collect()\
            })\
            .filter(|peers: &Vec<&AtlasArticleRow>| !peers.is_empty())\
            .unwrap_or_else(|| related_articles(claim, articles));\
        let mut title_support = 0u32;\
        let mut body_support = 0u32;\
        for peer in peers {\
            let claim_lower = claim.claim.to_ascii_lowercase();\
            let keywords: Vec<&str> = claim_lower.split_whitespace().map(|w| w.trim_matches(|c: char| !c.is_alphanumeric())).filter(|w| w.len() >= 4).collect();\
            \
            let title_lower = peer.title.to_ascii_lowercase();\
            let desc_lower = peer.description.to_ascii_lowercase();\
            \
            let title_both = contains_span(&peer.title, &claim.entity) && contains_span(&peer.title, &claim.object);\
            let body_both = contains_span(&desc_lower, &claim.entity.to_ascii_lowercase()) && contains_span(&desc_lower, &claim.object.to_ascii_lowercase());\
            \
            if title_both {\
                title_support += 1;\
            } else if body_both {\
                body_support += 1;\
            }\
        }\
        claim.title_peers = title_support;\
        claim.body_peers = body_support;\
    }\
}
