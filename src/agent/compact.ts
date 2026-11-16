// Keeping a conversation inside what a model can hold.
//
// A model reads the whole conversation on every request, and a long one stops fitting -
// soonest for a model on this machine, whose window is a few tens of thousands of tokens,
// most of it tool results nobody will ask about again. This shortens what is *sent*, not
// what is kept: the panel and the world still hold every message.
//
// In two steps, cheapest first. Old tool results are emptied - the call stays, so the
// model still knows it looked, but the kilobytes of JSON it got back go. Then whole early
// exchanges are dropped, from the start, a user turn at a time: an exchange is a user
// message and everything up to the next one, and it leaves together or not at all, so no
// tool result is ever left without the call it answers.

import type { AgentMessage } from "../ipc";

/** What an emptied tool result says in place of what it said. */
export const ELIDED = "[an earlier result, no longer kept in the conversation]";
/** Said in place of the exchanges that were dropped. */
export const DROPPED = "[earlier parts of this conversation were left out to make room]";

/** A message's size as the model will read it, in characters. */
export function weigh(message: AgentMessage): number {
  const calls = (message.tool_calls ?? []).reduce((sum, call) => sum + call.name.length + JSON.stringify(call.arguments ?? {}).length, 0);
  return (message.content?.length ?? 0) + calls + 16;
}

/**
 * The conversation, shortened to fit `budget` characters where it can be.
 *
 * The last exchange - the newest user message and whatever followed it - is never
 * touched: that is the turn in progress, and its tool results are the ones being read.
 */
export function compact(messages: AgentMessage[], budget: number): AgentMessage[] {
  const total = (list: AgentMessage[]) => list.reduce((sum, message) => sum + weigh(message), 0);
  if (total(messages) <= budget) return messages;

  let lastUser = -1;
  messages.forEach((message, index) => {
    if (message.role === "user") lastUser = index;
  });
  if (lastUser <= 0) return messages;

  // Step one: old tool results, emptied. Oldest first, stopping as soon as it fits.
  const sent = messages.map((message) => ({ ...message }));
  for (let index = 0; index < lastUser && total(sent) > budget; index += 1) {
    if (sent[index].role === "tool" && sent[index].content.length > ELIDED.length) {
      sent[index] = { ...sent[index], content: ELIDED };
    }
  }
  if (total(sent) <= budget) return sent;

  // Step two: whole exchanges, from the start.
  const starts = sent.map((message, index) => (message.role === "user" ? index : -1)).filter((index) => index >= 0 && index <= lastUser);
  let from = 0;
  for (const start of starts) {
    if (start === 0) continue;
    from = start;
    if (total(sent.slice(start)) + DROPPED.length <= budget) break;
  }
  if (from === 0) return sent;
  const kept = sent.slice(from);
  // Said to the model in the first message that is left, so it knows there was more.
  kept[0] = { ...kept[0], content: `${DROPPED}\n\n${kept[0].content}` };
  return kept;
}

// --- summarising -----------------------------------------------------------------------
//
// Dropping the oldest exchanges loses what was in them. Before it comes to that, the
// early part of a conversation is put to the model itself to be written down in a few
// lines, and that summary takes its place - in what is kept as well as in what is sent,
// since a conversation that is summarised on every request is one that pays for its whole
// length on every request.

/** How a summary's message begins, so it can be told from something the user typed. */
export const SUMMARY_MARK = "[Summary of the conversation so far]";

export const SUMMARY_SYSTEM =
  "You are compacting a conversation between a user and an assistant inside Geis, a protein-structure viewer. " +
  "Write a summary the assistant can continue from. Keep, in this order: what the user is trying to do; what is open " +
  "(molecules by their ids, chains); every change that was made and is still in effect (styles, colours, selections, " +
  "what was hidden); anything the user said they want or do not want; what was asked last and is not finished. " +
  "Use short lines. Leave out tool output that no longer matters and anything already undone. " +
  "Answer in the language the user wrote in. Do not call tools. Do not add anything that did not happen.";

/**
 * Where to cut a conversation that has outgrown its room: the early part to summarise,
 * and the recent part to keep word for word. Null when it still fits, or when there is
 * nothing before the last exchange to summarise.
 *
 * It is cut before a user message, so an exchange is whole on one side or the other, and
 * the recent side is at most about two fifths of the room - the summary and the turn
 * about to happen need the rest.
 */
export function splitForSummary(
  messages: AgentMessage[],
  budget: number,
): { early: AgentMessage[]; recent: AgentMessage[] } | null {
  const total = messages.reduce((sum, message) => sum + weigh(message), 0);
  if (total <= budget * 0.8) return null;
  const starts = messages.map((message, index) => (message.role === "user" ? index : -1)).filter((index) => index > 0);
  if (starts.length === 0) return null;
  let cut = starts[starts.length - 1];
  for (let at = starts.length - 1; at >= 0; at -= 1) {
    const size = messages.slice(starts[at]).reduce((sum, message) => sum + weigh(message), 0);
    if (size > budget * 0.4) break;
    cut = starts[at];
  }
  const early = messages.slice(0, cut);
  // A summary of nothing but an earlier summary is not worth a request.
  if (early.length < 2) return null;
  return { early, recent: messages.slice(cut) };
}

/** The early part as text for the summariser to read, long results cut short. */
export function transcript(messages: AgentMessage[], limit: number): string {
  const clip = (text: string, most: number) => (text.length > most ? `${text.slice(0, most)}… [cut]` : text);
  const lines = messages.map((message) => {
    if (message.role === "tool") return `RESULT: ${clip(message.content, 500)}`;
    const calls = (message.tool_calls ?? []).map((call) => `CALL ${call.name} ${clip(JSON.stringify(call.arguments ?? {}), 300)}`);
    const who = message.role === "user" ? "USER" : "ASSISTANT";
    return [message.content ? `${who}: ${clip(message.content, 2000)}` : "", ...calls].filter(Boolean).join("\n");
  });
  const text = lines.filter(Boolean).join("\n");
  // Too long even so: the end is kept, since it is the part closest to now.
  return text.length > limit ? `[the beginning is left out]\n${text.slice(text.length - limit)}` : text;
}

/** The message a summary is kept as. */
export function summaryMessage(text: string): AgentMessage {
  return { role: "user", content: `${SUMMARY_MARK}\n${text.trim()}`, tool_calls: [], tool_call_id: null };
}
