import React from "react";
import ReactDOM from "react-dom/client";

import App from "./App";
import "@/lib/i18n";
import { applyCodeAppearance, readCodeAppearance } from "@/lib/codeAppearance";
import { installDisableDefaultContextMenu } from "@/lib/disableDefaultContextMenu";
import { applyTheme, getThemePreference } from "@/lib/theme";
import "@/index.css";

applyTheme(getThemePreference());
applyCodeAppearance(readCodeAppearance());
installDisableDefaultContextMenu();

ReactDOM.createRoot(document.getElementById("root") as HTMLElement).render(
  <React.StrictMode>
    <App />
  </React.StrictMode>,
);
