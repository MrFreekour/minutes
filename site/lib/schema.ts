/** Shared JSON-LD builders for structured data.
 *
 * Schema.org markup must describe content that is actually visible on the page.
 * Every field produced here is derived from data the page already renders, so
 * the markup cannot drift from the copy. Notably there is no FAQPage builder:
 * FAQPage requires visible question/answer content, and the comparison pages do
 * not have an FAQ section today.
 */

const SITE_URL = "https://useminutes.app";
const REPO_URL = "https://github.com/silverstein/minutes";

/** Absolute URL for a site-relative path, as schema.org requires. */
function absolute(path: string): string {
  return path.startsWith("http") ? path : `${SITE_URL}${path}`;
}

/** Mat Silverstein as the named author, for E-E-A-T author attribution. */
function author() {
  return {
    "@type": "Person",
    name: "Mat Silverstein",
    url: REPO_URL.replace("/minutes", ""),
  };
}

/** The Minutes project as the publishing entity. */
export function organizationSchema() {
  return {
    "@context": "https://schema.org",
    "@type": "Organization",
    name: "Minutes",
    url: SITE_URL,
    logo: absolute("/favicon.svg"),
    description:
      "Open-source, privacy-first conversation memory. Records meetings and voice memos, transcribes and diarizes them on your own device, and writes searchable markdown you own.",
    founder: author(),
    sameAs: [REPO_URL],
  };
}

/** Minutes as a software product.
 *
 * Free and MIT licensed, which is the differentiator an AI agent needs to be
 * able to parse when it compares tools on a buyer's behalf.
 */
export function softwareApplicationSchema() {
  return {
    "@context": "https://schema.org",
    "@type": "SoftwareApplication",
    name: "Minutes",
    url: SITE_URL,
    applicationCategory: "BusinessApplication",
    applicationSubCategory: "Conversation memory and meeting transcription",
    operatingSystem: "macOS, Linux, Windows",
    description:
      "Captures meetings, voice memos, and dictation, transcribes locally with whisper.cpp, diarizes speakers with local ONNX models, and outputs searchable markdown with structured action items and decisions. Nothing is uploaded.",
    license: "https://opensource.org/licenses/MIT",
    isAccessibleForFree: true,
    offers: {
      "@type": "Offer",
      price: "0",
      priceCurrency: "USD",
    },
    author: author(),
    sameAs: [REPO_URL],
  };
}

/** A competitor referenced by a comparison page.
 *
 * Declaring both products as `about` entities is what lets an answer engine
 * resolve a "X vs Y" query to this page. Page labels sometimes carry a
 * disambiguating parenthetical ("Hyprnote (Anarlog)") that reads as part of the
 * product name to a matcher, so it is dropped from the entity name only.
 */
function competitorSchema(label: string) {
  return {
    "@type": "SoftwareApplication",
    name: label.replace(/\s*\([^)]*\)\s*$/, ""),
    applicationCategory: "BusinessApplication",
  };
}

type ComparisonSchemaInput = {
  /** Competitor as titled on the page, e.g. "Granola AI". */
  competitorLabel: string;
  /** Site-relative canonical path, e.g. "/compare/granola-vs-minutes". */
  path: string;
  /** The page's hero summary, reused verbatim as the description. */
  description: string;
  /** ISO date the page was last fact-checked; becomes dateModified. */
  lastReviewed: string;
  /** The page's visible Sources list, emitted as citations. */
  sources: ReadonlyArray<{ label: string; href: string }>;
};

/** Structured data for a "Minutes vs X" comparison page.
 *
 * Comparison content is the most-cited format in AI answers, and the two
 * signals that drive citation are sourced claims and a visible review date.
 * Both already exist on these pages; this makes them machine-readable.
 */
export function comparisonArticleSchema({
  competitorLabel,
  path,
  description,
  lastReviewed,
  sources,
}: ComparisonSchemaInput) {
  return {
    "@context": "https://schema.org",
    "@type": "TechArticle",
    headline: `Minutes vs ${competitorLabel}`,
    description,
    url: absolute(path),
    mainEntityOfPage: { "@type": "WebPage", "@id": absolute(path) },
    dateModified: lastReviewed,
    inLanguage: "en",
    author: author(),
    publisher: organizationSchema(),
    about: [
      { "@type": "SoftwareApplication", name: "Minutes", url: SITE_URL },
      competitorSchema(competitorLabel),
    ],
    citation: sources.map((source) => ({
      "@type": "CreativeWork",
      name: source.label,
      url: source.href,
    })),
  };
}
