import React, { useEffect, useState } from "react";

interface TourGuideProps {
  steps: {
    target?: string;
    title: string;
    content: string;
    position: "left" | "right" | "top" | "bottom" | "center";
    persistAfterReload?: boolean;
  }[];
  currentStep: number;
  onNext: () => void;
  onPrev: () => void;
  onClose: () => void;
  totalSteps: number;
  tourId?: string;
}

const TourGuide: React.FC<TourGuideProps> = ({
  steps,
  currentStep,
  onNext,
  onPrev,
  onClose,
  totalSteps,
  tourId = "default-tour",
}) => {
  const step = steps[currentStep];
  const targetElement = step.target
    ? document.querySelector(step.target)
    : null;
  const [position, setPosition] = useState({ top: 0, left: 0 });

  useEffect(() => {
    if (steps[currentStep]?.persistAfterReload) {
      localStorage.setItem(`tour-${tourId}-step`, currentStep.toString());
      localStorage.setItem(`tour-${tourId}-active`, "true");
    }
  }, [currentStep, steps, tourId]);

  useEffect(() => {
    if (step.position === "center") {
      // Center in the viewport
      const viewportHeight = window.innerHeight;
      const viewportWidth = window.innerWidth;
      setPosition({
        top: viewportHeight / 2 - 100, // Adjust for half the height of the tooltip
        left: viewportWidth / 2, // Use transform for horizontal centering
      });
    } else if (targetElement) {
      const rect = targetElement.getBoundingClientRect();
      let top = 0;
      let left = 0;

      switch (step.position) {
        case "right":
          top = rect.top;
          left = rect.right + 10;
          break;
        case "left":
          top = rect.top;
          left = rect.left - 320;
          break;
        case "bottom":
          top = rect.bottom + 10;
          left = rect.left;
          break;
        case "top":
          top = rect.top - 150;
          left = rect.left;
          break;
      }

      setPosition({ top, left });
      targetElement.classList.add("tour-highlight");
    }

    return () => {
      if (targetElement) {
        targetElement.classList.remove("tour-highlight");
      }
    };
  }, [currentStep, step.target, step.position]);

  return (
    <>
      <div className="fixed inset-0 bg-black bg-opacity-50 z-[970]" />
      <div
        className={`fixed z-[990] bg-white rounded-xl shadow-lg p-[1.2em] w-[30em] max-w-[90vw] ${
          step.position === "center"
            ? "transform -translate-x-1/2 -translate-y-1/2"
            : ""
        }`}
        style={{
          top: position.top,
          left: position.left,
        }}
      >
        <h3 className="font-medium text-lg mb-2">{step.title}</h3>
        <div
          className="text-gray-600 mb-4 whitespace-pre-wrap font-sans"
          style={{ maxWidth: "60ch", lineHeight: "1.5" }}
          dangerouslySetInnerHTML={{ __html: step.content }}
        />
        <div className="flex justify-between items-center">
          <div className="flex gap-2">
            {currentStep > 0 && (
              <button
                onClick={onPrev}
                className="px-3 py-1 rounded-full text-sm"
                style={{ color: "#007AFF" }}
              >
                Previous
              </button>
            )}
            {currentStep < totalSteps - 1 && (
              <button
                onClick={onNext}
                className="px-3 py-1 rounded-full text-sm"
                style={{ backgroundColor: "#007AFF", color: "white" }}
              >
                Next
              </button>
            )}
            {currentStep === totalSteps - 1 && (
              <button
                onClick={onClose}
                className="px-3 py-1 rounded-full text-sm"
                style={{ backgroundColor: "#007AFF", color: "white" }}
              >
                Finish
              </button>
            )}
          </div>
          <span className="text-sm text-gray-500">
            {currentStep + 1} of {totalSteps}
          </span>
        </div>
      </div>
    </>
  );
};

export default TourGuide;
