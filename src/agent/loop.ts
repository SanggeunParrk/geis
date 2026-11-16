// The conversation, as a loop.
//
// A turn is not one request: a model that can call tools answers, waits for results, and
// answers again, sometimes several times over. This is that loop and nothing else - it is
// handed a way to reach the model and a way to run a tool, and it does not know what
// either of them is. Pure enough to test, which matters, because the failure modes here
// are the kind that only show up on the tenth conversation: a tool result attached to the
// wrong call, a refusal that leaves the model waiting forever, a loop that never ends.

import type { AgentDelta, AgentMessage, AgentToolCall, ChatReply } from "../ipc";

export interface LoopHooks {
  /**
   * Sends the conversation so far and returns what came back, reporting the pieces as they
   * arrive so the panel can show the model at work.
   */
  complete: (messages: AgentMessage[], onDelta: (delta: AgentDelta) => void) => Promise<ChatReply>;
  /**
   * Runs one call and returns what to tell the model. A refusal and a failure are both
   * results, not exceptions: the model has to be told, or it waits for an answer that
   * never comes.
   */
  runTool: (call: AgentToolCall) => Promise<string>;
  /** Each message as it joins the conversation, for the panel to draw. */
  onMessage?: (message: AgentMessage) => void;
  /** A piece of the answer as it is written. */
  onDelta?: (delta: AgentDelta) => void;
  /** A turn of the loop is about to ask the model again. */
  onStep?: (step: number) => void;
}

/**
 * How many times round the loop before giving up.
 *
 * A model that has misunderstood a tool will call it again with the same arguments for as
 * long as it is allowed to. Twelve is enough for a real task - read the graph, read a
 * document, make three changes, report - and short enough that a stuck one is noticed.
 */
export const MAX_STEPS = 12;

export interface LoopResult {
  /** Everything added to the conversation this turn. */
  added: AgentMessage[];
  /** Why it finished: the model was done, or it ran out of steps, or it failed. */
  stopped: "end" | "steps" | "error";
  error?: string;
  /** Tokens, summed over every request this turn. */
  usage: { input: number; output: number };
}

export function assistantMessage(reply: ChatReply): AgentMessage {
  return {
    role: "assistant",
    content: reply.text,
    // For the panel to show. The readable text is not what goes back to the model...
    thinking: reply.thinking,
    tool_calls: reply.tool_calls,
    tool_call_id: null,
    // ...this is: the turn exactly as the provider wrote it, where the provider wants it
    // back that way. Claude signs its reasoning and expects each turn returned intact; a
    // turn rebuilt from its text and calls alone loses the thinking behind a tool call.
    ...(reply.provider === undefined || reply.provider === null ? {} : { provider: reply.provider }),
  };
}

export function toolMessage(call: AgentToolCall, content: string): AgentMessage {
  return { role: "tool", content, tool_calls: [], tool_call_id: call.id };
}

/**
 * Runs one turn to completion: model, tools, model, until it stops asking for tools.
 *
 * `history` is the conversation up to and including the user's message; the messages this
 * adds are returned rather than pushed into it, so the caller decides what to keep.
 */
export async function converse(
  history: AgentMessage[],
  hooks: LoopHooks,
  maxSteps: number = MAX_STEPS,
): Promise<LoopResult> {
  // A copy, because the caller may well be appending what `onMessage` hands it to the very
  // array it passed in - and then every request would carry each tool exchange twice.
  const base = [...history];
  const added: AgentMessage[] = [];
  const usage = { input: 0, output: 0 };
  const push = (message: AgentMessage) => {
    added.push(message);
    hooks.onMessage?.(message);
  };

  for (let step = 0; step < maxSteps; step += 1) {
    let reply: ChatReply;
    hooks.onStep?.(step);
    try {
      reply = await hooks.complete([...base, ...added], (delta) => hooks.onDelta?.(delta));
    } catch (error) {
      return { added, stopped: "error", error: String(error), usage };
    }
    usage.input += reply.usage.input;
    usage.output += reply.usage.output;
    push(assistantMessage(reply));

    if (reply.tool_calls.length === 0) return { added, stopped: "end", usage };

    // Results go back in the order they were asked for, each naming its call: a model that
    // asked for three things at once has to be able to tell which answer is which.
    for (const call of reply.tool_calls) {
      let content: string;
      try {
        content = await hooks.runTool(call);
      } catch (error) {
        content = `error: ${String(error)}`;
      }
      push(toolMessage(call, content));
    }
  }
  // Out of steps with the model still asking. Said as a result rather than thrown, so the
  // conversation stays valid and the next thing the user types still works.
  return { added, stopped: "steps", usage };
}
