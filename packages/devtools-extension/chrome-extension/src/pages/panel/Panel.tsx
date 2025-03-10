import React, { useState } from "react";
import "@pages/panel/Panel.css";
import Chat from "@pages/panel/components/Chat";
import Settings from "@pages/panel/components/Settings";
import CLI from "@pages/panel/components/CLI";

export default function Panel() {
  const [currentPage, setPage] = useState("CLI");
  const nav = (pageName: string) => {
    setPage(pageName);
  };
  const renderPage = () => {
    switch (currentPage) {
      case "CLI": {
        return <CLI nav={nav} />;
      }
      case "Settings": {
        return <Settings nav={nav} />;
      }
    }
  };
  return <div className="container">{renderPage()}</div>;
}
