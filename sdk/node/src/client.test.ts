import { describe, it } from 'node:test';
import * as assert from 'node:assert/strict';
import { Plasmate } from './index';

type ToolCall = { name: string; args: Record<string, unknown> };

describe('read selectors', () => {
  it('forwards selector options through fetch and text reads', async () => {
    const browser = new Plasmate({ binary: 'unused' });
    const calls: ToolCall[] = [];
    const client = browser as unknown as {
      callTool: (name: string, args: Record<string, unknown>) => Promise<unknown>;
    };
    client.callTool = async (name, args) => {
      calls.push({ name, args });
      return name === 'extract_text' ? 'text' : {};
    };

    await browser.fetchPage('fixture', { selector: 'main' });
    await browser.som('fixture', { selector: 'interactive' });
    await browser.extractText('fixture', { selector: 'content' });

    assert.deepEqual(calls, [
      { name: 'fetch_page', args: { url: 'fixture', selector: 'main' } },
      { name: 'fetch_page', args: { url: 'fixture', selector: 'interactive' } },
      { name: 'extract_text', args: { url: 'fixture', selector: 'content' } },
    ]);
    browser.close();
  });
});
