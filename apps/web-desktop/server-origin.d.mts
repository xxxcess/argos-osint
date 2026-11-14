import type { Agent } from "node:http";

export function rustOrigin(): string;
export function rustUpstream(): Promise<{
  origin: string;
  authorization?: string;
}>;
export function directAgent(target: URL): Agent;
export function configuredWebOrigin(): string | undefined;
export function configuredWebBasePath(): string;
export function readProtectedTokenFile(path?: string): Promise<string>;
export function incomingWebOrigin(
  host: string,
  protocol: string,
  legacyPort?: number,
): string;
