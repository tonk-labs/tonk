import dotenv from 'dotenv';
import { createGroq } from '@ai-sdk/groq';
import { generateText } from 'ai';

dotenv.config({ path: '../.env' });

const groq = createGroq({
  apiKey: process.env.VITE_GROQ_API_KEY || ''
});

async function test() {
  try {
    console.log('Testing with llama model...');
    const result = await generateText({
      model: groq('llama-3.3-70b-versatile'),
      messages: [{ role: 'user', content: 'Say hello' }]
    });
    console.log('Success! Response:', result.text);
  } catch (error) {
    console.error('Error with llama:', error.message);
  }

  try {
    console.log('\nTesting with kimi model...');
    const result = await generateText({
      model: groq('moonshotai/kimi-k2-instruct'),
      messages: [{ role: 'user', content: 'Say hello' }]
    });
    console.log('Success! Response:', result.text);
  } catch (error) {
    console.error('Error with kimi:', error.message);
  }
}

test();
