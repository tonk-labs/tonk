import { Wrench } from 'lucide-react';
import type React from 'react';
import type { ToolCall } from './types';

interface ToolCallDetailsProps {
  toolCalls: ToolCall[];
}

const ToolCallDetails: React.FC<ToolCallDetailsProps> = ({ toolCalls }) => {
  if (!toolCalls || toolCalls.length === 0) return null;

  return (
    <details className="mt-2">
      <summary className="cursor-pointer text-[10px] text-gray-500 hover:text-gray-700">
        View tool call details ({toolCalls.length} calls)
      </summary>
      <div className="mt-2 space-y-1">
        {toolCalls.map(tool => (
          <div
            key={tool.id}
            className="bg-gray-50 border border-gray-200 rounded p-1.5"
          >
            <div className="text-[10px] text-blue-700 font-medium flex items-center gap-1">
              <Wrench className="w-2.5 h-2.5" />
              {tool.name}
            </div>
            <div className="mt-1 pl-3 space-y-1">
              <div className="text-[9px] text-gray-600">
                <strong>Args:</strong>
                <pre className="bg-white p-1 rounded mt-0.5 overflow-x-auto text-[8px]">
                  {JSON.stringify(tool.args, null, 2)}
                </pre>
              </div>
              {tool.result && (
                <div className="text-[9px] text-gray-600">
                  <strong>Result:</strong>
                  <pre className="bg-white p-1 rounded mt-0.5 overflow-x-auto max-h-32 overflow-y-auto text-[8px]">
                    {JSON.stringify(tool.result, null, 2)}
                  </pre>
                </div>
              )}
            </div>
          </div>
        ))}
      </div>
    </details>
  );
};

export default ToolCallDetails;
