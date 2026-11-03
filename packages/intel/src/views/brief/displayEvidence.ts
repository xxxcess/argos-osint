/** Drop the stored assessment tag, for example `[claim · supports]`. */
const ASSESSMENT_TAG = /^\[(?:claim|inference) · [^\]]+\]\s*/i;

export function displayEvidence(text: string): string {
  return text.replace(ASSESSMENT_TAG, "");
}
