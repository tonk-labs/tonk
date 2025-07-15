import { Link } from "react-router-dom";

/**
 * A simple Hello World view component that demonstrates basic layout and styling
 */
const HelloWorld = () => {
  return (
    <main className="min-h-screen bg-gray-50 p-4 md:p-6 lg:p-8">
      <section className="max-w-2xl mx-auto">
        <div className="bg-white rounded-lg shadow-sm p-6">
          <div className="mb-4">
            <h1 className="text-3xl font-bold text-gray-900">Hello World</h1>
          </div>
          <div className="space-y-4">
            <p className="text-gray-600">
              Welcome to your new Tonk application!
            </p>
            <div>
              <Link 
                to="/api-test" 
                className="inline-block bg-blue-500 hover:bg-blue-600 text-white px-4 py-2 rounded-md transition-colors"
              >
                Test API Proxy →
              </Link>
            </div>
          </div>
        </div>
      </section>
    </main>
  );
};

export default HelloWorld;
