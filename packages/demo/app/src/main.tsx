import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import { install } from "@twind/core";
import config from "../twind.config.js";
import "./index.css";
import App from "./App.tsx";

install(config);

createRoot(document.getElementById("root")!).render(
  <StrictMode>
    <App />
  </StrictMode>,
);
