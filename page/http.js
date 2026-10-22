// pair-ui's messages over HTTP to the pair-ui host that served the page,
// with the token from the page URL's `pair` parameter.

import { answer } from "./messages.js";

const POLL_RETRY_MS = 2000;

export async function httpHost(token) {
  const call = (method, route, body) =>
    fetch(`/_pair/${route}`, {
      method,
      headers: { Authorization: `Bearer ${token}`, "Content-Type": "application/json" },
      body: body === undefined ? undefined : JSON.stringify(body),
    });
  const read = async (route) => {
    const response = await call("GET", route);
    if (!response.ok) throw new Error(`the pair-ui host answered ${response.status} to ${route}`);
    return response.json();
  };
  const [{ project }, kept] = await Promise.all([read("project"), read("recall")]);

  async function poll(handlers) {
    for (;;) {
      const response = await call("GET", "next").catch(() => null);
      if (!response?.ok) {
        await new Promise((resolve) => setTimeout(resolve, POLL_RETRY_MS));
        continue;
      }
      if (response.status === 204) continue;
      const answered = await answer(handlers, await response.json());
      if (answered) await call("POST", "answer", answered).catch(() => {});
    }
  }

  return {
    project,
    opensLinks: true,
    kept,
    start: (handlers) => void poll(handlers),
    keep: (view) => void call("PUT", "keep", view ?? null).catch(() => {}),
  };
}
