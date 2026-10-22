// A reference page for pair-ui: answers tells and reveals through the page
// client, and keeps its one piece of state (a click count) across reloads.
// `pair` is a link to the client in `page/`, so the build serves it.

import { connect } from "./pair/index.js";

const button = document.getElementById("click");
const log = (line) => (document.getElementById("log").textContent += line + "\n");
const show = () => (button.textContent = String(clicks));

const host = await connect();
if (!host) throw new Error("open this page through a pair-ui host");
let clicks = host.kept?.clicks ?? 0;
show();

button.onclick = () => {
  clicks += 1;
  show();
  host.keep({ clicks });
};

host.start({
  // The bodies this page takes: { "get": "state" } and { "set": n }.
  tell(body) {
    log(`told ${JSON.stringify(body)}`);
    if (body?.get === "state") return { ok: true, message: `${clicks} clicks`, result: { clicks } };
    if (!Number.isInteger(body?.set)) throw new Error(`this page does not take ${JSON.stringify(body)}`);
    clicks = body.set;
    show();
    host.keep({ clicks });
    return { ok: true, message: `set to ${clicks}` };
  },
  // The monikers this page takes: `demo://<project>/clicks/<n>`.
  reveal(moniker) {
    log(`revealed ${moniker}`);
    const n = Number(moniker.match(/^demo:\/\/[^/]+\/clicks\/(\d+)$/)?.[1] ?? NaN);
    if (!Number.isInteger(n)) throw new Error(`not a moniker this page shows: ${moniker}`);
    clicks = n;
    show();
    host.keep({ clicks });
    return { ok: true, message: `showing ${clicks} clicks` };
  },
});
