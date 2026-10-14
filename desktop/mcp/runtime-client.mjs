// Compatibility export for existing local clients and smoke tests.
import { sharedModule } from './shared.mjs';
export const { connectRuntime } = await import(sharedModule('runtime-client.mjs'));
