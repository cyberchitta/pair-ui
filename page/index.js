// The page's side of pair-ui. `connect()` finds the host the page runs
// under: the pair-ui host over HTTP when the URL carries a `pair` token,
// a framing host over postMessage when the page is in a frame, or none.

import { frameHost } from "./frame.js";
import { httpHost } from "./http.js";

export async function connect() {
  const token = new URLSearchParams(location.search).get("pair");
  if (token) return httpHost(token);
  if (window.parent !== window) return frameHost();
  return null;
}
