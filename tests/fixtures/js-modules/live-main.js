import { value, increment } from './live-dependency.js';
increment();
globalThis.liveBindingResult = value;
