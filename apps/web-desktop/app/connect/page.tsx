import { configuredWebBasePath } from "../../server-origin.mjs";
import { safeReturnPath } from "../../connect-session.mjs";
import { ConnectForm } from "./connect-form";

export const dynamic = "force-dynamic";

export default async function ConnectPage({
  searchParams,
}: {
  searchParams: Promise<Record<string, string | string[] | undefined>>;
}) {
  const query = await searchParams;
  return (
    <ConnectForm
      returnPath={safeReturnPath(query.return)}
      basePath={configuredWebBasePath()}
    />
  );
}
