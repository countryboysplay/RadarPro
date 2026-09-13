import React from "react";
import ReactDOM from "react-dom/client";
import App from "./App";
import { attachDesktopLogging } from "./platform/desktop";

// S10 desktop shell: forward this webview's console output into the native
// log file when running inside Tauri; a complete no-op in a plain browser
// tab. See `src/platform/desktop.ts`'s doc comment.
void attachDesktopLogging();

ReactDOM.createRoot(document.getElementById("root") as HTMLElement).render(
  <React.StrictMode>
    <App />
  </React.StrictMode>,
);
