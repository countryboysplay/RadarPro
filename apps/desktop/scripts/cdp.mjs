// Usage: node cdp.mjs eval "<js expression>"  |  node cdp.mjs shot out.png
const [mode, arg] = process.argv.slice(2);
const pages = await (await fetch("http://localhost:9222/json")).json();
const page = pages.find((p) => p.type === "page" && p.title === "RadarPro");
const ws = new WebSocket(page.webSocketDebuggerUrl);
await new Promise((r) => ws.addEventListener("open", r));
let id = 0;
const send = (method, params) =>
  new Promise((resolve) => {
    const myId = ++id;
    ws.addEventListener("message", function on(e) {
      const msg = JSON.parse(e.data);
      if (msg.id !== myId) return;
      ws.removeEventListener("message", on);
      resolve(msg.result ?? msg);
    });
    ws.send(JSON.stringify({ id: myId, method, params }));
  });
if (mode === "eval") {
  const r = await send("Runtime.evaluate", { expression: arg, awaitPromise: true, returnByValue: true });
  console.log(JSON.stringify(r.result?.value ?? r.exceptionDetails ?? r, null, 1));
} else {
  const r = await send("Page.captureScreenshot", { format: "png" });
  (await import("node:fs")).writeFileSync(arg, Buffer.from(r.data, "base64"));
  console.log("saved", arg);
}
ws.close();
