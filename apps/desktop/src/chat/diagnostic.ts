/**
 * The text "Copy diagnostic details" puts on the clipboard for a stopped
 * channel or a stopped chat session: what a support request needs to find the
 * failure, and nothing from the conversation itself. `detail` is the agent's
 * own description of the failed check, which the pane does not show.
 */
export function chatDiagnostic({
  team,
  teamId,
  server,
  channel,
  code,
  detail,
  at,
}: {
  team: string;
  /** The team ID the chat scope carries, when known. */
  teamId?: string;
  server: string;
  /** The stopped channel's ID; omitted when the whole session stopped. */
  channel?: string;
  code: string;
  detail?: string;
  /** Milliseconds since the epoch. */
  at: number;
}): string {
  return [
    channel ? 'FOKS chat: channel stopped' : 'FOKS chat: chat stopped',
    `Team: ${team}${teamId ? ` (${teamId})` : ''}`,
    `Server: ${server}`,
    ...(channel ? [`Channel: ${channel}`] : []),
    `Error code: ${code}`,
    ...(detail ? [`Detail: ${detail}`] : []),
    `Time: ${new Date(at).toISOString()}`,
  ].join('\n');
}
