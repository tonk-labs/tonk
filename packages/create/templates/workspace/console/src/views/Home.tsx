import React, { useState } from "react";
import FileBrowser from "./FileBrowser";

/**
 * Home view component with tabbed interface
 * 
 * Features three tabs:
 * 1. Files - displays the FileBrowser component
 * 2. Tab2 - placeholder tab
 * 3. Tab3 - placeholder tab
 */
const Home = () => {
  const [activeTab, setActiveTab] = useState(0);

  const tabs = [
    { id: 0, label: "Files", component: <FileBrowser /> },
    { 
      id: 1, 
      label: "Tab2", 
      component: (
        <div className="p-8 text-center">
          <h2 className="text-2xl font-semibold text-gray-900 mb-4">Tab 2 Content</h2>
          <p className="text-gray-600">This is a placeholder for Tab 2 content.</p>
        </div>
      )
    },
    { 
      id: 2, 
      label: "Tab3", 
      component: (
        <div className="p-8 text-center">
          <h2 className="text-2xl font-semibold text-gray-900 mb-4">Tab 3 Content</h2>
          <p className="text-gray-600">This is a placeholder for Tab 3 content.</p>
        </div>
      )
    }
  ];

  return (
    <main className="min-h-screen bg-gray-50">
      <div className="max-w-7xl mx-auto">
        {/* Tab Navigation */}
        <nav className="bg-white shadow-sm border-b border-gray-200" role="tablist">
          <div className="flex space-x-0">
            {tabs.map((tab) => (
              <button
                key={tab.id}
                role="tab"
                aria-selected={activeTab === tab.id}
                aria-controls={`tabpanel-${tab.id}`}
                onClick={() => setActiveTab(tab.id)}
                className={`px-6 py-4 text-sm font-medium border-b-2 transition-colors duration-200 ${
                  activeTab === tab.id
                    ? "border-blue-500 text-blue-600 bg-blue-50"
                    : "border-transparent text-gray-500 hover:text-gray-700 hover:border-gray-300"
                }`}
              >
                {tab.label}
              </button>
            ))}
          </div>
        </nav>

        {/* Tab Content */}
        <div className="bg-white">
          {tabs.map((tab) => (
            <div
              key={tab.id}
              id={`tabpanel-${tab.id}`}
              role="tabpanel"
              aria-labelledby={`tab-${tab.id}`}
              className={activeTab === tab.id ? "block" : "hidden"}
            >
              {tab.component}
            </div>
          ))}
        </div>
      </div>
    </main>
  );
};

export default Home;
