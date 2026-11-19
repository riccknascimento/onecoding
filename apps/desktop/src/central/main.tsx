import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import "../styles.css";
import "./central.css";
import { CentralApp } from "./CentralApp";

createRoot(document.getElementById("root")!).render(
  <StrictMode>
    <CentralApp />
  </StrictMode>,
);
