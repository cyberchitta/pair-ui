// Answers one message from the host with the page's handlers. Both
// transports carry the same messages, keyed `pair: 1`; this is the part they
// share. A handler that throws is a refusal, so the page changes nothing and
// says why.

export async function answer(handlers, message) {
  const { reveal, tell } = message;
  if (reveal) return settle(reveal.id, () => revealWith(handlers, reveal));
  if (tell) return settle(tell.id, () => handlers.tell(tell.body));
  return null;
}

function revealWith(handlers, { moniker, state }) {
  if (!handlers.reveal) throw new Error("this page takes no reveals");
  return handlers.reveal(moniker, state);
}

async function settle(id, run) {
  try {
    return { id, ...(await run()) };
  } catch (e) {
    return { id, ok: false, message: String(e instanceof Error ? e.message : e) };
  }
}
