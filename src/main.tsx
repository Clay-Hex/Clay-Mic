import React from "react";
import ReactDOM from "react-dom/client";
import App from "./App";
import { OverlayWindow } from "./overlay/OverlayWindow";
import { VoiceIndicator } from "./overlay/VoiceIndicator";
import "./styles/globals.css";

const windowKind = new URLSearchParams(window.location.search).get("window");

// Disable the WebView's native right-click menu everywhere.
window.addEventListener("contextmenu", (event) => event.preventDefault());

if (windowKind === "overlay" || windowKind === "indicator") {
  document.documentElement.style.background = "transparent";
  document.body.style.background = "transparent";
}

ReactDOM.createRoot(document.getElementById("root")!).render(
  <React.StrictMode>
    {windowKind === "overlay" ? (
      <OverlayWindow />
    ) : windowKind === "indicator" ? (
      <VoiceIndicator />
    ) : (
      <App />
    )}
  </React.StrictMode>
);
