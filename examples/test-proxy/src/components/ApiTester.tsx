import React, { useState } from "react";

interface ApiResponse {
  success: boolean;
  data?: any;
  error?: string;
  message?: string;
}

interface RequestLog {
  id: number;
  method: string;
  url: string;
  status: number;
  response: ApiResponse;
  timestamp: string;
}

export const ApiTester: React.FC = () => {
  const [requestLogs, setRequestLogs] = useState<RequestLog[]>([]);
  const [loading, setLoading] = useState<string | null>(null);
  const [newUserData, setNewUserData] = useState({
    name: "",
    email: "",
    role: "user"
  });

  const logRequest = (method: string, url: string, status: number, response: ApiResponse) => {
    const newLog: RequestLog = {
      id: Date.now(),
      method,
      url,
      status,
      response,
      timestamp: new Date().toLocaleTimeString()
    };
    setRequestLogs(prev => [newLog, ...prev]);
  };

  const makeRequest = async (method: string, endpoint: string, body?: any) => {
    setLoading(endpoint);
    try {
      const options: RequestInit = {
        method,
        headers: {
          'Content-Type': 'application/json',
        },
      };
      
      if (body) {
        options.body = JSON.stringify(body);
      }

      const response = await fetch(endpoint, options);
      const data = await response.json();
      
      logRequest(method, endpoint, response.status, data);
    } catch (error) {
      logRequest(method, endpoint, 0, { 
        success: false, 
        error: error instanceof Error ? error.message : 'Unknown error' 
      });
    } finally {
      setLoading(null);
    }
  };

  const handleCreateUser = (e: React.FormEvent) => {
    e.preventDefault();
    makeRequest('POST', '/api/users', newUserData);
    setNewUserData({ name: "", email: "", role: "user" });
  };

  const clearLogs = () => {
    setRequestLogs([]);
  };

  return (
    <div className="max-w-6xl mx-auto p-6 space-y-6">
      <div className="bg-white rounded-lg shadow-md p-6">
        <h1 className="text-3xl font-bold text-gray-800 mb-6">API Proxy Tester</h1>
        
        <div className="grid grid-cols-1 md:grid-cols-2 lg:grid-cols-3 gap-4 mb-6">
          <button
            onClick={() => makeRequest('GET', '/api/users')}
            disabled={loading === '/api/users'}
            className="bg-blue-500 hover:bg-blue-600 disabled:bg-blue-300 text-white px-4 py-2 rounded-md transition-colors"
          >
            {loading === '/api/users' ? 'Loading...' : 'Get All Users'}
          </button>
          
          <button
            onClick={() => makeRequest('GET', '/api/users/1')}
            disabled={loading === '/api/users/1'}
            className="bg-green-500 hover:bg-green-600 disabled:bg-green-300 text-white px-4 py-2 rounded-md transition-colors"
          >
            {loading === '/api/users/1' ? 'Loading...' : 'Get User #1'}
          </button>
          
          <button
            onClick={() => makeRequest('GET', '/api/users/999')}
            disabled={loading === '/api/users/999'}
            className="bg-yellow-500 hover:bg-yellow-600 disabled:bg-yellow-300 text-white px-4 py-2 rounded-md transition-colors"
          >
            {loading === '/api/users/999' ? 'Loading...' : 'Get User #999 (404)'}
          </button>
          
          <button
            onClick={() => makeRequest('GET', '/api/stats')}
            disabled={loading === '/api/stats'}
            className="bg-purple-500 hover:bg-purple-600 disabled:bg-purple-300 text-white px-4 py-2 rounded-md transition-colors"
          >
            {loading === '/api/stats' ? 'Loading...' : 'Get Stats'}
          </button>
          
          <button
            onClick={() => makeRequest('GET', '/api/health')}
            disabled={loading === '/api/health'}
            className="bg-indigo-500 hover:bg-indigo-600 disabled:bg-indigo-300 text-white px-4 py-2 rounded-md transition-colors"
          >
            {loading === '/api/health' ? 'Loading...' : 'Health Check'}
          </button>
        </div>

        <div className="bg-gray-50 rounded-lg p-4 mb-6">
          <h3 className="text-lg font-semibold text-gray-700 mb-3">Create New User</h3>
          <form onSubmit={handleCreateUser} className="space-y-3">
            <div className="grid grid-cols-1 md:grid-cols-3 gap-3">
              <input
                type="text"
                placeholder="Name"
                value={newUserData.name}
                onChange={(e) => setNewUserData(prev => ({ ...prev, name: e.target.value }))}
                className="border border-gray-300 rounded-md px-3 py-2 focus:outline-none focus:ring-2 focus:ring-blue-500"
                required
              />
              <input
                type="email"
                placeholder="Email"
                value={newUserData.email}
                onChange={(e) => setNewUserData(prev => ({ ...prev, email: e.target.value }))}
                className="border border-gray-300 rounded-md px-3 py-2 focus:outline-none focus:ring-2 focus:ring-blue-500"
                required
              />
              <select
                value={newUserData.role}
                onChange={(e) => setNewUserData(prev => ({ ...prev, role: e.target.value }))}
                className="border border-gray-300 rounded-md px-3 py-2 focus:outline-none focus:ring-2 focus:ring-blue-500"
              >
                <option value="user">User</option>
                <option value="admin">Admin</option>
              </select>
            </div>
            <button
              type="submit"
              disabled={loading === '/api/users'}
              className="bg-red-500 hover:bg-red-600 disabled:bg-red-300 text-white px-4 py-2 rounded-md transition-colors"
            >
              {loading === '/api/users' ? 'Creating...' : 'Create User'}
            </button>
          </form>
        </div>
      </div>

      <div className="bg-white rounded-lg shadow-md p-6">
        <div className="flex justify-between items-center mb-4">
          <h2 className="text-2xl font-semibold text-gray-800">Request Logs</h2>
          <button
            onClick={clearLogs}
            className="bg-gray-500 hover:bg-gray-600 text-white px-4 py-2 rounded-md transition-colors"
          >
            Clear Logs
          </button>
        </div>
        
        {requestLogs.length === 0 ? (
          <p className="text-gray-500 text-center py-8">No requests made yet. Try clicking some buttons above!</p>
        ) : (
          <div className="space-y-4 max-h-96 overflow-y-auto">
            {requestLogs.map((log) => (
              <div key={log.id} className="border border-gray-200 rounded-lg p-4">
                <div className="flex justify-between items-start mb-2">
                  <div className="flex items-center space-x-2">
                    <span className={`px-2 py-1 rounded text-xs font-semibold ${
                      log.method === 'GET' ? 'bg-blue-100 text-blue-800' :
                      log.method === 'POST' ? 'bg-green-100 text-green-800' :
                      'bg-gray-100 text-gray-800'
                    }`}>
                      {log.method}
                    </span>
                    <span className="font-mono text-sm text-gray-600">{log.url}</span>
                  </div>
                  <div className="flex items-center space-x-2">
                    <span className={`px-2 py-1 rounded text-xs font-semibold ${
                      log.status >= 200 && log.status < 300 ? 'bg-green-100 text-green-800' :
                      log.status >= 400 ? 'bg-red-100 text-red-800' :
                      'bg-gray-100 text-gray-800'
                    }`}>
                      {log.status || 'ERROR'}
                    </span>
                    <span className="text-xs text-gray-500">{log.timestamp}</span>
                  </div>
                </div>
                <div className="bg-gray-50 rounded p-3">
                  <pre className="text-sm text-gray-700 whitespace-pre-wrap overflow-x-auto">
                    {JSON.stringify(log.response, null, 2)}
                  </pre>
                </div>
              </div>
            ))}
          </div>
        )}
      </div>
    </div>
  );
};