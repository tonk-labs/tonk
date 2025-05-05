import React, { useState } from "react";
import { readDoc } from '@tonk/keepsync'

/**
 * StoreViewer component that allows users to enter a store name 
 * and view its contents in a pretty-printed format
 */
const StoreViewer = () => {
  const [storeId, setStoreId] = useState<string>("");
  const [storeData, setStoreData] = useState<any>(null);
  const [error, setError] = useState<string | null>(null);

  const handleInputChange = (e: React.ChangeEvent<HTMLInputElement>) => {
    setStoreId(e.target.value);
  };

  const handleSubmit = async (e: React.FormEvent) => {
    e.preventDefault();
    
    if (!storeId.trim()) {
      setError("Please enter a store ID");
      return;
    }

    try {
      const doc = await readDoc(storeId)
      setStoreData(doc);
      setError(null);
    } catch (err) {
      setError(`Error fetching store: ${err instanceof Error ? err.message : String(err)}`);
      setStoreData(null);
    }
  };

  return (
    <main className="min-h-screen bg-gray-50 p-4 md:p-6 lg:p-8">
      <section className="max-w-4xl mx-auto">
        <div className="bg-white rounded-lg shadow-sm p-6">
          <div className="mb-6">
            <h1 className="text-3xl font-bold text-gray-900">Doc Viewer</h1>
            <p className="text-gray-600 mt-2">
              Enter a document path to view its contents
            </p>
          </div>

          <form onSubmit={handleSubmit} className="mb-6">
            <div className="flex gap-2">
              <input
                type="text"
                value={storeId}
                onChange={handleInputChange}
                placeholder="Enter store ID"
                className="flex-1 px-4 py-2 border border-gray-300 rounded-md focus:outline-none focus:ring-2 focus:ring-blue-500"
                aria-label="Store ID"
              />
              <button
                type="submit"
                className="px-4 py-2 bg-blue-600 text-white rounded-md hover:bg-blue-700 focus:outline-none focus:ring-2 focus:ring-blue-500 focus:ring-offset-2"
                aria-label="View store"
              >
                View
              </button>
            </div>
          </form>

          {error && (
            <div className="mb-6 p-4 bg-red-100 border border-red-300 rounded-md text-red-700">
              {error}
            </div>
          )}

          <div className="bg-gray-100 rounded-md p-4 h-[calc(100vh-300px)] overflow-auto">
            {storeData ? (
              <pre className="text-sm text-gray-800 whitespace-pre-wrap">
                {JSON.stringify(storeData, null, 2)}
              </pre>
            ) : (
              <div className="text-gray-500 text-center p-4">
                {error ? "No data to display" : "Enter a store ID and click View to see store data"}
              </div>
            )}
          </div>
        </div>
      </section>
    </main>
  );
};

export default StoreViewer; 