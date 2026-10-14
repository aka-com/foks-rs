/**
 * The text "Copy diagnostic details" puts on the clipboard for a stopped
 * channel or a stopped chat session: what a support request needs to find the
 * failure, and nothing from the conversation itself. `detail` is the agent's
 * own description of the failed check, which the pane does not show. A channel
 * blocked with no failure, because `sync-inbox` listed it in
 * `blocked_channels`, has no code; `reportedBy` names that source instead.
 */
export function chatDiagnostic({
  team,
  teamId,
  server,
  channel,
  code,
  reportedBy,
  detail,
  at,
}: {
  team: string;
  /** The team ID the chat scope carries, when known. */
  teamId?: string;
  server: string;
  /** The stopped channel's ID; omitted when the whole session stopped. */
  channel?: string;
  code?: string;
  /** Where a block with no failure came from. */
  reportedBy?: string;
  detail?: string;
  /** Milliseconds since the epoch. */
  at: number;
}): string {
  return [
    channel ? 'FOKS chat: channel stopped' : 'FOKS chat: chat stopped',
    `Team: ${team}${teamId ? ` (${teamId})` : ''}`,
    `Server: ${server}`,
    ...(channel ? [`Channel: ${channel}`] : []),
    ...(code ? [`Error code: ${code}`] : []),
    ...(reportedBy ? [`Reported by: ${reportedBy}`] : []),
    ...(detail ? [`Detail: ${detail}`] : []),
    `Time: ${new Date(at).toISOString()}`,
  ].join('\n');
}
