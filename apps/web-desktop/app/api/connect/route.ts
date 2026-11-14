import {
  createConnectionResponse,
  currentConnectionResponse,
} from "../../../connect-session.mjs";

export const runtime = "nodejs";
export const dynamic = "force-dynamic";
export const GET = currentConnectionResponse;
export const POST = createConnectionResponse;
