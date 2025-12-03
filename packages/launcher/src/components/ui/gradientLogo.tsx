export default function GradientLogo() {
  return (
    <div className="relative">
      <svg
        width="150"
        height="150"
        viewBox="0 0 300 300"
        fill="none"
        xmlns="http://www.w3.org/2000/svg"
        className="p-4 relative z-10 dark:invert-100"
      >
        <defs>
          <linearGradient id="logo-gradient" x1="0" y1="0" x2="100%" y2="0%">
            <stop offset="0%" stopColor="#e6c767">
              <animate
                attributeName="stop-color"
                values="#e6c767; #8dbec8; #a8d8b9; #f4a7a7; #e6c767"
                dur="4s"
                repeatCount="indefinite"
              />
            </stop>
            <stop offset="50%" stopColor="#a8d8b9">
              <animate
                attributeName="stop-color"
                values="#a8d8b9; #f4a7a7; #e6c767; #8dbec8; #a8d8b9"
                dur="4s"
                repeatCount="indefinite"
              />
            </stop>
            <stop offset="100%" stopColor="#e6c767">
              <animate
                attributeName="stop-color"
                values="#e6c767; #8dbec8; #a8d8b9; #f4a7a7; #e6c767"
                dur="4s"
                repeatCount="indefinite"
              />
            </stop>
          </linearGradient>
        </defs>
        <path
          fillRule="evenodd"
          clipRule="evenodd"
          d="M300 0V300H150C232.843 300 300 232.843 300 150H0C0 232.843 67.1573 300 150 300H0V0H300ZM75 30C50.1472 30 30 50.1472 30 75C30 99.8528 50.1472 120 75 120C99.8528 120 120 99.8528 120 75C120 50.1472 99.8528 30 75 30ZM225 30C200.147 30 180 50.1472 180 75C180 99.8528 200.147 120 225 120C249.853 120 270 99.8528 270 75C270 50.1472 249.853 30 225 30Z"
          style={{ fill: 'url(#logo-gradient)' }}
        />
      </svg>
      <div className="absolute inset-0 bg-linear-to-tr from-blue-500/20 via-purple-500/20 to-pink-500/20 blur-2xl rounded-full opacity-50  animate-pulse" />
    </div>
  );
}
