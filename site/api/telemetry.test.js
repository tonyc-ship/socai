import assert from 'node:assert/strict';
import test from 'node:test';

import { __testing } from './telemetry.js';

test('sanitizeEvent strips fields that should not reach Axiom', () => {
  const sanitized = __testing.sanitizeEvent({
    event: 'socai_cli_tool_trace',
    install_id: 'install-1',
    daemon_session_id: 'legacy-session-1',
    request_id: 'request-1',
    command: 'topic_scan',
    tool_name: 'topic_scan',
    query_text_enabled: false,
    metadata: {
      num_notes: 12,
      tab: 'latest',
      debug_snapshot: true,
    },
    arch: 'arm64',
    created_at_ms: 1,
    client_created_at_ms: 2,
    received_at_ms: 3,
    query_redacted: true,
    num_notes: 99,
    tab_label: 'should_drop',
  });

  assert.equal(sanitized.install_id, 'install-1');
  assert.equal(sanitized.session_id, 'legacy-session-1');
  assert.equal(sanitized.request_id, 'request-1');
  assert.equal(sanitized.command, 'topic_scan');
  assert.equal(sanitized.tool_name, 'topic_scan');
  assert.deepEqual(sanitized.metadata, {
    num_notes: 12,
    tab: 'latest',
    debug_snapshot: true,
  });

  for (const key of [
    'event',
    'arch',
    'created_at_ms',
    'client_created_at_ms',
    'received_at_ms',
    'query_redacted',
    'num_notes',
    'tab_label',
    'daemon_session_id',
  ]) {
    assert.equal(Object.hasOwn(sanitized, key), false, `${key} should be stripped`);
  }
});

test('sanitizeEvent preserves shallow primitive metadata and rejects unsafe metadata', () => {
  const sanitized = __testing.sanitizeEvent({
    event: 'socai_cli_tool_trace',
    install_id: 'install-1',
    command: 'topic_scan',
    metadata: {
      num_notes: 5,
      tab: ' latest ',
      enabled: true,
      null_value: null,
      nested: { unsafe: true },
      array: ['unsafe'],
      nan_value: Number.NaN,
      'bad key': 'unsafe',
      'bad$key': 'unsafe',
    },
  });

  assert.deepEqual(sanitized.metadata, {
    num_notes: 5,
    tab: 'latest',
    enabled: true,
    null_value: null,
  });
});

test('sanitizeEvent rejects missing or non-socai event names', () => {
  assert.equal(__testing.sanitizeEvent({ command: 'topic_scan' }), null);
  assert.equal(
    __testing.sanitizeEvent({ event: 'other_event', command: 'topic_scan' }),
    null,
  );
});

test('sanitizeEvent flattens legacy properties without leaking disallowed fields', () => {
  const sanitized = __testing.sanitizeEvent({
    event: 'socai_cli_tool_trace',
    distinct_id: 'install-from-distinct',
    properties: {
      request_id: 'request-1',
      command: 'search_notes',
      query_text: '  Bloc1 V4  ',
      created_at_ms: 123,
      metadata: { tab: 'discover' },
    },
  });

  assert.equal(sanitized.install_id, 'install-from-distinct');
  assert.equal(sanitized.request_id, 'request-1');
  assert.equal(sanitized.command, 'search_notes');
  assert.equal(sanitized.query_text, 'Bloc1 V4');
  assert.deepEqual(sanitized.metadata, { tab: 'discover' });
  assert.equal(Object.hasOwn(sanitized, 'event'), false);
  assert.equal(Object.hasOwn(sanitized, 'created_at_ms'), false);
});
