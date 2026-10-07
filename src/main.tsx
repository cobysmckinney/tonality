import React from "react";
import ReactDOM from "react-dom/client";
import "@fontsource-variable/geist";
import "./styles.css";
import App from "./App";
import { ErrorBoundary } from "./components/Problems";

ReactDOM.createRoot(document.getElementById("root") as HTMLElement).render(
  <React.StrictMode>
    <ErrorBoundary>
      <App />
    </ErrorBoundary>
  </React.StrictMode>,
);
