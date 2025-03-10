import React, { useState } from "react";
import { createRoot } from "react-dom/client";
import CLI from "@pages/panel/components/CLI";
import Settings from "@pages/panel/components/Settings";
import "@pages/test-view/tailwind.css";

const App = () => {
  const [currentPage, setCurrentPage] = useState<string>("CLI");

  const handleNav = (pageName: string) => {
    setCurrentPage(pageName);
  };

  return (
    <div className="app-container">
      {currentPage === "CLI" && <CLI nav={handleNav} />}
      {currentPage === "Settings" && <Settings nav={handleNav} />}
    </div>
  );
};

function init() {
  const container = document.getElementById("__root");
  const root = createRoot(container!);

  root.render(<App />);
}

init();
