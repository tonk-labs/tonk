import { createTonkAgent } from './src/mastra/agent.js';

async function test() {
  try {
    console.log('Creating agent...');
    const agent = await createTonkAgent();
    console.log('Agent created');
    
    const messages = [{ role: 'user', content: 'Say hello' }];
    const runtimeContext = new Map();
    runtimeContext.set('userName', 'Developer');
    
    console.log('Creating stream...');
    const stream = await agent.stream(messages, {
      maxSteps: 5,
      runtimeContext,
      onStepFinish: ({ toolCalls, toolResults }) => {
        console.log('Step finished - tools:', toolCalls?.length || 0);
      }
    });
    
    console.log('Stream created, reading chunks...');
    let count = 0;
    for await (const chunk of stream.textStream) {
      count++;
      console.log(`Chunk ${count}:`, chunk);
    }
    console.log('Total chunks:', count);
    
  } catch (error) {
    console.error('Error:', error);
    console.error('Stack:', error.stack);
  }
}

test();
