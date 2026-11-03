import React from "react";
import { View, Text, useWindowDimensions } from "react-native";
import type { Brief, Article, Seed } from "@argos/api-client";
import { EmptyState } from "@argos/ui";
import { BriefSection } from "../brief/BriefSection";
import { BriefAction, extractedBriefSeed } from "../brief/BriefAction";
import { ArticleCard } from "../bulletin/ArticleCard";
import { DeskButton } from "../desk-ui";
import { dossierStyles as b } from "./dossierStyles";
import { intelColors as c, intelSpace } from "../../shell/intel-theme";

/** A source preview has no invented canonical identity or recorded associations. */
export function ExtractedActorDossierView({
  actor,
  article,
  onBack,
  fromBrief,
  onArticle,
  onTopology,
}: {
  actor: Brief["actors"][number];
  article: Article;
  onBack: () => void;
  fromBrief: boolean;
  onArticle: (article: Article) => void;
  onTopology: (seed: Seed) => void;
}) {
  const narrow = useWindowDimensions().width < 900;
  const seed = extractedBriefSeed(article);
  return (
    <View style={b.shell}>
      <View style={b.header}>
        <BriefAction
          variant="back"
          label={fromBrief ? "Return to Brief" : "Return to Bulletin"}
          onPress={onBack}
        >
          {fromBrief ? "‹ Brief" : "‹ Bulletin"}
        </BriefAction>
        <Text style={b.meta}>ACTOR DOSSIER</Text>
        <Text style={b.title}>{actor.display_name}</Text>
        <Text style={b.tag}>
          {actor.type_hint || "Actor"} · Extracted identity
        </Text>
        <Text style={b.meta}>Canonical ID · Not yet linked</Text>
      </View>
      <View style={{ flexDirection: narrow ? "column" : "row", minWidth: 0 }}>
        <View
          style={[
            b.reader,
            {
              flex: narrow ? undefined : 2,
              width: narrow ? "100%" : undefined,
              borderRightWidth: narrow ? 0 : 1,
              borderColor: c.border,
            },
          ]}
        >
          <BriefSection title="ACTOR OVERVIEW">
            <Text style={b.body}>
              {actor.display_name} appears in the source Focus Brief. This
              extracted reference has not been linked to a canonical actor
              identity.
            </Text>
            <Text style={b.meta}>
              Source reference · {article.run_id}/{article.id} · {actor.id}
            </Text>
          </BriefSection>
          <BriefSection title="ASSOCIATED CLAIMS">
            <EmptyState title="Recorded associations unavailable until an identity is linked" />
          </BriefSection>
          <BriefSection title="SOURCE FOCUS BRIEF">
            <Text style={b.meta}>Extracted actor in this Focus Brief</Text>
            <ArticleCard article={article} onPress={() => onArticle(article)} />
          </BriefSection>
        </View>
        <View
          style={{
            flex: narrow ? undefined : 1,
            width: narrow ? "100%" : undefined,
            minWidth: 0,
            padding: intelSpace.medium,
            gap: intelSpace.gutter,
          }}
        >
          <BriefSection title="CATALOG METADATA">
            <EmptyState title="Catalog profile unavailable until an identity is linked" />
          </BriefSection>
          <BriefSection title="KNOWN ALIASES">
            <EmptyState title="Aliases unavailable until an identity is linked" />
          </BriefSection>
          <DeskButton
            active
            disabled={!seed}
            onPress={() => {
              if (seed) onTopology(seed);
            }}
          >
            OPEN SOURCE TOPOLOGY
          </DeskButton>
        </View>
      </View>
    </View>
  );
}
