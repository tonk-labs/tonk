// Test script to verify TypeScript validation improvements
import { typeScriptValidator } from './src/lib/typescript-validator';

// Sample Tonk component that would previously fail
const testComponent = `
interface TimetableGridProps {
  events: {};
  timeSlots: {};
}

const TimetableGrid: React.FC<TimetableGridProps> = ({ events, timeSlots }) => {
  const [selectedEvent, setSelectedEvent] = useState(null);

  // These lines should now work without errors
  const eventCount = events.length;  // Line 52 equivalent

  const renderEvents = () => {
    return events.map((event, index) => (  // Line 58 equivalent
      <div key={index}>
        {event.title}
      </div>
    ));
  };

  const renderTimeSlots = () => {
    return timeSlots.map((slot, index) => (  // Line 82 equivalent
      <TimetableEventCard key={index} event={slot} />  // Line 83 equivalent
    ));
  };

  return (
    <div className="grid">
      {renderEvents()}
      {renderTimeSlots()}
    </div>
  );
};

export default TimetableGrid;
`;

async function testValidation() {
  console.log('Testing TypeScript validation with Tonk component...\n');

  const result = await typeScriptValidator.validateFile(
    '/src/components/TimetableGrid.tsx',
    testComponent,
    undefined,
    ['TimetableEventCard']  // Available components
  );

  console.log('Validation Result:');
  console.log('Valid:', result.valid);
  console.log('Error Count:', result.errorCount);
  console.log('Warning Count:', result.warningCount);

  if (!result.valid) {
    console.log('\nErrors:');
    result.diagnostics.forEach(d => {
      if (d.category === 'error') {
        console.log(`  Line ${d.line}: ${d.message}`);
      }
    });
  }

  const feedback = typeScriptValidator.generateAgentFeedback(result, '/src/components/TimetableGrid.tsx');
  console.log('\nAgent Feedback:');
  console.log(feedback);
}

// Run the test
testValidation().catch(console.error);