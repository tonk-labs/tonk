import { useState, useEffect } from "react";
import "./App.css";
import { useVFS } from "./hooks/useVFS";
import type { DocumentData } from "@tonk/core";

function App() {
  return (
    <main className="p-4">
      <div className="text-3xl font-bold">
        some <span className="underline">times</span> this works
      </div>
    </main>
  );
}

export default App;
