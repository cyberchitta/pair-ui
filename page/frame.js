// pair-ui's messages over postMessage, for a host that puts the page in a
// frame. Such a host serves each project's page from an origin whose host is
// the project's name, and opens no links from it.

import { answer } from "./messages.js";

const RECALL_WAIT_MS = 300;

const post = (message) => window.parent.postMessage({ pair: 1, ...message }, "*");

export async function frameHost() {
  const early = [];
  let handle = (message) => early.push(message);
  let recalled = () => {};
  window.addEventListener("message", (event) => {
    if (event.source !== window.parent || event.data?.pair !== 1) return;
    if ("recalled" in event.data) recalled(event.data.recalled);
    else handle(event.data);
  });
  const kept = await new Promise((resolve) => {
    recalled = resolve;
    setTimeout(() => resolve(null), RECALL_WAIT_MS);
    post({ recall: true });
  });
  return {
    project: location.host,
    opensLinks: false,
    kept,
    start(handlers) {
      handle = async (message) => {
        const answered = await answer(handlers, message);
        if (answered) post({ answer: answered });
      };
      early.splice(0).forEach(handle);
      post({ ready: true });
    },
    keep: (view) => post({ keep: view ?? null }),
  };
}
