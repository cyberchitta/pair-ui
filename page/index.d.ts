/** What the page says back to a tell or a reveal. `ok: false` means the
 * page changed nothing, and `message` says why. */
export type Answer = { ok: boolean; message: string; result?: unknown };

/** The page's answers. A reveal asks it to show a moniker
 * (`<scheme>://<project>…`) with the caller's `state`; a tell hands it a
 * body only the page and its app's commands understand. Leave out `reveal`
 * and every reveal is refused. A handler that throws refuses with the
 * error's message. */
export type Handlers = {
  reveal?(moniker: string, state: unknown): Answer | Promise<Answer>;
  tell(body: unknown): Answer | Promise<Answer>;
};

export type Host = {
  /** The project the page belongs to: monikers name it. */
  project: string;
  /** Whether a link can open in a new tab. A framed page opens none. */
  opensLinks: boolean;
  /** What the page last kept, or null. */
  kept: unknown;
  /** Says the page can answer, and starts answering. Call once, after the
   * kept view is shown: the host holds reveals until then. */
  start(handlers: Handlers): void;
  /** Replaces what is kept (JSON, under 64 KB); null forgets it. */
  keep(view: unknown): void;
};

/** The host this page runs under, or null when it has none (opened as a
 * plain file). */
export function connect(): Promise<Host | null>;
