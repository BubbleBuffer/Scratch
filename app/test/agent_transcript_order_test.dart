import 'package:flutter_test/flutter_test.dart';
import 'package:notes_app/addons/agent/api.dart';

const _at = '2026-10-08T00:00:00Z';

AgentMessage _msg(
  String id,
  String role, {
  String text = 'text',
  List<AgentToolCall> toolCalls = const [],
  String? toolCallId,
}) => AgentMessage(
  id: id,
  role: role,
  content: text,
  text: text,
  toolCalls: toolCalls,
  toolCallId: toolCallId,
  isError: false,
  createdAt: _at,
);

AgentSessionState _replay({
  required List<AgentMessage> messages,
  List<AgentRunRecord> runs = const [],
}) => AgentSessionState(
  session: const AgentSession(
    id: 'replay',
    title: 'Replay',
    createdAt: _at,
    updatedAt: _at,
  ),
  seq: 0,
  liveSeen: 0,
  streamingText: '',
  pendingApprovals: const [],
  runs: runs,
  activity: const [],
  edits: const [],
  messages: messages,
);

AgentRunRecord _run(String id) => AgentRunRecord(
  id: id,
  sessionId: 'replay',
  model: 'test',
  status: 'completed',
  startedAt: _at,
  endedAt: _at,
);

void main() {
  test('equal timestamps retain durable user and assistant message order', () {
    const at = '2026-10-08T00:00:00Z';
    final state = AgentSessionState(
      session: const AgentSession(
        id: 'replay',
        title: 'Replay',
        createdAt: at,
        updatedAt: at,
      ),
      seq: 0,
      liveSeen: 0,
      streamingText: '',
      pendingApprovals: const [],
      runs: const [],
      activity: const [],
      edits: const [],
      messages: [
        for (var i = 0; i < 3; i++) ...[
          AgentMessage(
            id: 'u$i',
            role: 'user',
            content: 'Question $i',
            text: 'Question $i',
            toolCalls: const [],
            isError: false,
            createdAt: at,
          ),
          AgentMessage(
            id: 'a$i',
            role: 'assistant',
            content: 'Answer $i',
            text: 'Answer $i',
            toolCalls: const [],
            isError: false,
            createdAt: at,
          ),
        ],
      ],
    );
    final items = agentTurns(state);
    expect(
      items.map((item) => item.id),
      ['u0', 'message-a0', 'u1', 'message-a1', 'u2', 'message-a2'],
    );
  });

  test('a run keeps its place among equal-time users, at its first message', () {
    final state = _replay(
      runs: [_run('r')],
      messages: [
        _msg('u0', 'user'),
        _msg('a0', 'assistant'),
        _msg('u1', 'user'),
        _msg('a1', 'assistant'),
      ],
    );
    expect(agentTurns(state).map((item) => item.id), ['u0', 'r', 'u1']);
  });

  test('split pieces interleave with equal-time users by message position', () {
    final state = _replay(
      runs: [_run('r')],
      messages: [
        _msg('u0', 'user'),
        _msg('a0', 'assistant'),
        _msg('u1', 'user'),
        _msg('a1', 'assistant'),
      ],
    );
    final items = agentTurns(
      state,
      segmentOf: const {'a0': '0.0', 'a1': '1.0'},
    );
    expect(items.map((item) => item.id), ['u0', 'r#0.0', 'u1', 'r#1.0']);
  });

  test('a run with no message follows every message at its time', () {
    final state = _replay(
      runs: [_run('r')],
      messages: [_msg('u0', 'user'), _msg('u1', 'user')],
    );
    expect(agentTurns(state).map((item) => item.id), ['u0', 'u1', 'r']);
  });
}
