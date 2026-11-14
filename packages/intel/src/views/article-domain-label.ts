import {
  INTEL_CATEGORIES,
  INTEL_DOMAIN_NAMES,
  type Article,
  type StrategicDomain,
} from "@argos/api-client";

/** Use strategic domain labels for the current admitted projection. */
export function articleDomainLabel(article: Article): string {
  const domains = article.categories;
  const current =
    article.taxonomy_version === "strategic-domains-v1" &&
    article.admission_state === "admitted" &&
    article.publication_version === "atlas-admission-publication-v4" &&
    domains &&
    domains.length >= 1 &&
    domains.length <= 2 &&
    (domains.length === 1 || domains[0] !== domains[1]) &&
    domains[0] === article.category &&
    domains.every((id) => INTEL_CATEGORIES.includes(id as StrategicDomain));
  if (current) return INTEL_DOMAIN_NAMES[article.category as StrategicDomain];
  return `Legacy category · ${article.category.toUpperCase() || "UNAVAILABLE"}`;
}
