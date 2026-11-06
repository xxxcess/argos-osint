import {
  INTEL_CATEGORIES,
  INTEL_DOMAIN_NAMES,
  type Article,
  type StrategicDomain,
} from "@argos/api-client";

/** Labels require the server's current admitted projection, including its exact ordered pair. */
export function articleDomainLabel(article: Article): string {
  const pair = article.categories;
  const current =
    article.taxonomy_version === "strategic-domains-v1" &&
    article.admission_state === "admitted" &&
    article.publication_version === "atlas-admission-publication-v3" &&
    pair?.length === 2 &&
    pair[0] !== pair[1] &&
    pair[0] === article.category &&
    pair.every((id) => INTEL_CATEGORIES.includes(id as StrategicDomain));
  if (current) return INTEL_DOMAIN_NAMES[article.category as StrategicDomain];
  return `Legacy category · ${article.category.toUpperCase() || "UNAVAILABLE"}`;
}
