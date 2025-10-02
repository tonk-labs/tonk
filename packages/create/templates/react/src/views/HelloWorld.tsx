/**
 * A simple Hello World view component
 */
const HelloWorld = () => {
  return (
    <main className="min-h-screen bg-gray-50 p-4 md:p-6 lg:p-8">
      <section className="max-w-4xl mx-auto">
        <div className="bg-white rounded-lg shadow-sm p-6">
          <div className="mb-6">
            <h1 className="text-3xl font-bold text-gray-900">Hello World</h1>
          </div>

          <div className="mb-8">
            <p className="text-gray-600 mb-4">
              Welcome to your new tonk! This is a simple Hello World example.
            </p>
          </div>

          <div className="bg-blue-50 rounded-lg p-6 border border-blue-200">
            <h2 className="text-xl font-semibold text-blue-900 mb-2">
              🎉 Congratulations!
            </h2>
            <p className="text-blue-700">
              Your tonk is running successfully. You can start building it by
              editing the components in the{' '}
              <code className="bg-blue-100 px-2 py-1 rounded">src/</code>{' '}
              directory.
            </p>
          </div>
        </div>
      </section>
    </main>
  );
};

export default HelloWorld;
