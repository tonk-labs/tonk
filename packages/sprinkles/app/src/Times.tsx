import { Link } from 'wouter';

function Times() {
  return (
    <div className="min-h-screen bg-[#f9f7f1] p-8">
      <div className="max-w-6xl mx-auto">
        {/* Navigation */}
        <div className="mb-6">
          <Link href="/" className="text-sm hover:underline">
            ← Return to Home
          </Link>
        </div>

        {/* Header with decorative border */}
        <div className="border-t-4 border-b-4 border-black py-4 mb-8">
          <div className="text-center">
            <div className="text-xs tracking-widest mb-2">
              VOL. CLXXIII . . . No. 60,042
            </div>
            <h1 className="font-serif text-6xl font-bold tracking-tight mb-2">
              The Tonk Times East
            </h1>
            <div className="text-xs tracking-widest">
              "All the News That's Fit to Print"
            </div>
            <div className="text-sm mt-2">
              NEW YORK, THURSDAY, OCTOBER 16, 2025
            </div>
          </div>
        </div>

        {/* Main headline */}
        <div className="border-b border-black pb-6 mb-6">
          <h2 className="font-serif text-5xl font-bold leading-tight mb-4">
            MODERN WEB FRAMEWORKS REVOLUTIONIZE DEVELOPMENT PRACTICES
          </h2>
          <p className="font-serif text-xl leading-relaxed">
            Industry Experts Announce Breakthrough in Component-Based
            Architecture. Routing Technologies Advance. React Maintains
            Dominance.
          </p>
        </div>

        {/* Three column layout */}
        <div className="grid grid-cols-3 gap-8 border-b border-black pb-6 mb-6">
          <div className="border-r border-black pr-6">
            <h3 className="font-serif text-2xl font-bold mb-3">
              Vite Build Tool Gains Widespread Adoption
            </h3>
            <p className="font-serif text-sm leading-relaxed mb-3">
              SILICON VALLEY - The revolutionary build tool continues its
              meteoric rise, offering developers unprecedented speed and
              efficiency in their daily workflows.
            </p>
            <p className="font-serif text-sm leading-relaxed mb-3">
              Engineers across the nation report significant productivity gains,
              with some claiming build times reduced by over 90 percent compared
              to legacy systems.
            </p>
            <p className="font-serif text-sm leading-relaxed">
              Industry analysts predict continued growth throughout the
              remainder of the decade.
            </p>
          </div>

          <div className="border-r border-black pr-6">
            <h3 className="font-serif text-2xl font-bold mb-3">
              Tailwind CSS Declared New Standard
            </h3>
            <p className="font-serif text-sm leading-relaxed mb-3">
              WASHINGTON - In a landmark decision, the utility-first CSS
              framework has been officially recognized as the preferred styling
              methodology by leading technology firms.
            </p>
            <p className="font-serif text-sm leading-relaxed mb-3">
              "This represents a paradigm shift in how we approach web design,"
              remarked one prominent frontend architect at a major tech company.
            </p>
            <p className="font-serif text-sm leading-relaxed">
              The framework's fourth major version introduces even more powerful
              capabilities.
            </p>
          </div>

          <div>
            <h3 className="font-serif text-2xl font-bold mb-3">
              Wouter Router Praised for Simplicity
            </h3>
            <p className="font-serif text-sm leading-relaxed mb-3">
              BOSTON - The minimalist routing library has captured the attention
              of developers seeking lightweight alternatives to more complex
              solutions.
            </p>
            <p className="font-serif text-sm leading-relaxed mb-3">
              Weighing in at mere kilobytes, the library demonstrates that
              powerful functionality need not come at the cost of bundle size.
            </p>
            <p className="font-serif text-sm leading-relaxed">
              "Sometimes less truly is more," noted a senior engineer at a
              startup.
            </p>
          </div>
        </div>

        {/* Secondary stories */}
        <div className="grid grid-cols-2 gap-8">
          <div>
            <h4 className="font-serif text-xl font-bold mb-2">
              TypeScript Adoption Reaches All-Time High
            </h4>
            <p className="font-serif text-sm leading-relaxed">
              Survey data reveals that type safety has become a non-negotiable
              requirement for modern web applications. The language continues to
              evolve with quarterly updates.
            </p>
          </div>

          <div>
            <h4 className="font-serif text-xl font-bold mb-2">
              React 19 Features Debut to Acclaim
            </h4>
            <p className="font-serif text-sm leading-relaxed">
              The latest version introduces server components and improved
              concurrent rendering, marking yet another milestone in the
              framework's storied history.
            </p>
          </div>
        </div>

        {/* Footer */}
        <div className="mt-12 pt-6 border-t border-black text-center text-xs">
          <p>Weather: Fair and Pleasant • Temperature: 68° • Sunset: 6:14 PM</p>
        </div>
      </div>
    </div>
  );
}

export default Times;
