import React, { useState } from "react";
import { importLocationsFromKeepsync } from "../utils/importLocations";
import { Loader, Download, CheckCircle, AlertCircle } from "lucide-react";

interface ImportLocationsButtonProps {
  docPath?: string;
  username?: string;
  onComplete?: (result: {
    success: boolean;
    imported: number;
    skipped: number;
    failed: number;
    message: string;
  }) => void;
}

const ImportLocationsButton: React.FC<ImportLocationsButtonProps> = ({
  docPath = "/jack/locations",
  username = "Jack",
  onComplete,
}) => {
  const [isImporting, setIsImporting] = useState(false);
  const [result, setResult] = useState<{
    success: boolean;
    imported: number;
    skipped: number;
    failed: number;
    message: string;
  } | null>(null);

  // Apple design system colors
  const appleColors = {
    blue: "#007AFF",
    green: "#34C759",
    red: "#FF3B30",
    yellow: "#FFCC00",
    gray: {
      light: "#F2F2F7",
      medium: "#E5E5EA",
      dark: "#8E8E93",
    },
  };

  const handleImport = async () => {
    setIsImporting(true);
    setResult(null);

    try {
      const importResult = await importLocationsFromKeepsync(docPath, username);
      setResult(importResult);

      if (onComplete) {
        onComplete(importResult);
      }
    } catch (error) {
      console.error("Import error:", error);
      setResult({
        success: false,
        imported: 0,
        skipped: 0,
        failed: 0,
        message: `Error: ${error}`,
      });
    } finally {
      setIsImporting(false);
    }
  };

  return (
    <div className="flex flex-col items-center">
      <button
        onClick={handleImport}
        disabled={isImporting}
        className="mb-1 w-full py-2 px-3 text-sm rounded-lg font-medium flex items-center justify-center gap-2"
        style={{
          backgroundColor: appleColors.blue,
          color: "white",
        }}
      >
        {isImporting ? (
          <>
            <Loader size={16} className="animate-spin mr-2" />
            Importing...
          </>
        ) : (
          <>
            <Download size={16} className="mr-2" />
            Import Saved Locations
          </>
        )}
      </button>

      {result && (
        <div
          className="mt-3 p-3 rounded-lg text-sm flex items-start"
          style={{
            backgroundColor: result.success
              ? "rgba(52, 199, 89, 0.1)"
              : "rgba(255, 59, 48, 0.1)",
            color: result.success ? appleColors.green : appleColors.red,
            maxWidth: "280px",
            backdropFilter: "blur(10px)",
            WebkitBackdropFilter: "blur(10px)",
            border: `1px solid ${
              result.success
                ? "rgba(52, 199, 89, 0.2)"
                : "rgba(255, 59, 48, 0.2)"
            }`,
          }}
        >
          {result.success ? (
            <CheckCircle size={16} className="mr-2 mt-0.5 flex-shrink-0" />
          ) : (
            <AlertCircle size={16} className="mr-2 mt-0.5 flex-shrink-0" />
          )}
          <span>{result.message}</span>
        </div>
      )}
    </div>
  );
};

export default ImportLocationsButton;
