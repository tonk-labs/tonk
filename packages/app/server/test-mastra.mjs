import dotenv from 'dotenv';
import { createGroq } from '@ai-sdk/groq';
import { Agent } from '@mastra/core/agent';

dotenv.config({ path: '../.env' });

const groq = createGroq({
  apiKey: process.env.VITE_GROQ_API_KEY || ''
});

const model = groq('moonshotai/kimi-k2-instruct');

const agent = new Agent({
  name: 'Test Agent',
  instructions: 'You are a helpful assistant',
  model: model,
  tools: {}
});

async function test() {
  try {
    console.log('Testing Mastra agent stream...');
    const messages = [{ role: 'user', content: 'Say hello' }];
    
    const stream = await agent.stream(messages, {
      maxSteps: 5,
      onStepFinish: (step) => {
        console.log('Step finished:', step);
      }
    });
    
    console.log('Stream created, reading text...');
    let fullText = '';
    for await (const chunk of stream.textStream) {
      console.log('Chunk:', chunk);
      fullText += chunk;
    }
    console.log('Full response:', fullText);
  } catch (error) {
    console.error('Error:', error);
    console.error('Stack:', error.stack);
  }
}

test();
