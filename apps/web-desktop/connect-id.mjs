/** One browser/Node parser, checked against the Rust Connect ID fixture corpus.
 * The opaque workstation ID is display context, never authentication authority.
 */
export function parseConnectId(input) {
  if (
    typeof input !== "string" ||
    input.length < 1 ||
    input.length > 128 ||
    /[^A-Za-z0-9_.-]/u.test(input)
  )
    throw new Error("Enter a valid registered workstation Connect ID");
  // Preserve the backend-issued spelling. IPs and workspace names do not
  // participate in identity; legacy IP:workspace IDs fail the grammar above.
  return { connectId: input, workstationId: input };
}
