import dotenv from 'dotenv';
import { createGroq } from '@ai-sdk/groq';

dotenv.config({ path: '../.env' });

const groq = createGroq({
  apiKey: process.env.VITE_GROQ_API_KEY || ''
});

const model = groq('llama-3.3-70b-versatile'); // Try a different model

async function test() {
  try {
    console.log('Testing Groq API...');
    const result = await model.doGenerate({
      inputFormat: 'messages',
      mode: { type: 'regular' },
      messages: [
        { role: 'user', content: [{ type: 'text', text: 'Say hello' }] }
      ]
    });
    console.log('Success! Response:', result.text);
  } catch (error) {
    console.error('Error:', error.message);
    if (error.response) {
      console.error('Response status:', error.response.status);
      console.error('Response data:', error.response.data);
    }
  }
}

test();
