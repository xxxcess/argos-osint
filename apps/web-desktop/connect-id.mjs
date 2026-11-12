/** One browser/Node parser, checked against the Rust Connect ID fixture corpus. */
export function parseConnectId(input) {
  const bytes = new TextEncoder();
  if (typeof input !== "string" || bytes.encode(input).length > 176)
    throw new Error("Enter a valid registered Connect ID");
  let ip;
  let workspace;
  if (input.startsWith("[")) {
    const end = input.indexOf("]:");
    if (end < 0) throw new Error("IPv6 Connect IDs require brackets");
    const raw = input.slice(1, end);
    if (!raw.includes(":") || raw.includes("%"))
      throw new Error("Enter a valid workstation IP address");
    try {
      ip = new URL(`http://[${raw}]/`).hostname.slice(1, -1).toLowerCase();
    } catch {
      throw new Error("Enter a valid workstation IP address");
    }
    const mapped = /^::ffff:([0-9a-f]{1,4}):([0-9a-f]{1,4})$/u.exec(ip);
    if (mapped) {
      const high = Number.parseInt(mapped[1], 16);
      const low = Number.parseInt(mapped[2], 16);
      ip = `${high >> 8}.${high & 255}.${low >> 8}.${low & 255}`;
    }
    workspace = input.slice(end + 2);
  } else {
    if (input.split(":").length !== 2)
      throw new Error("IPv6 Connect IDs require brackets");
    [ip, workspace] = input.split(":");
  }
  if (!ip.includes(":")) {
    const octets = ip.split(".");
    if (
      octets.length !== 4 ||
      octets.some(
        (value) => !/^(0|[1-9][0-9]{0,2})$/u.test(value) || Number(value) > 255,
      )
    )
      throw new Error("Enter a valid workstation IP address");
    const first = Number(octets[0]);
    if (
      ip === "0.0.0.0" ||
      first === 127 ||
      (first >= 224 && first <= 239) ||
      ip === "255.255.255.255"
    )
      throw new Error("Select a non-loopback unicast workstation address");
  } else if (ip === "::" || ip === "::1" || ip.startsWith("ff")) {
    throw new Error("Select a non-loopback unicast workstation address");
  }
  if (
    !workspace ||
    bytes.encode(workspace).length > 128 ||
    /^\p{White_Space}|\p{White_Space}$/u.test(workspace) ||
    workspace.startsWith(".") ||
    /[/\\:\p{Cc}\uD800-\uDFFF]/u.test(workspace)
  )
    throw new Error("Enter a valid registered workspace basename");
  workspace = workspace.normalize("NFC");
  if (bytes.encode(workspace).length > 128)
    throw new Error("Workspace name exceeds the byte limit");
  const connectId = `${ip.includes(":") ? `[${ip}]` : ip}:${workspace}`;
  return { connectId, ip, workspaceName: workspace };
}
