// Grouped turns split at outline boundaries: each piece holds its own texts
// and cells, a tool call's work sits with its call, and no whole turn sticks
// to one side of a boundary. Pure Dart, no widgets.

import 'package:agent_kit/agent_kit.dart'
    show Outline, OutlineChapter, OutlineLayout;
import 'package:flutter_test/flutter_test.dart';
import 'package:notes_app/addons/agent/api.dart';

const _at = '2026-10-09T12:00:00Z';

// One run covers the assistant messages a1, a2 and a3, so without boundaries
// they group into one turn. a1 calls c1 (answered by t1); a3 calls c3.
AgentSessionState _session() => AgentSessionState(
  session: const AgentSession(
    id: 'chat',
    title: 'Chat',
    createdAt: _at,
    updatedAt: _at,
  ),
  seq: 0,
  liveSeen: 0,
  streamingText: '',
  pendingApprovals: const [],
  runs: const [
    AgentRunRecord(
      id: 'run',
      sessionId: 'chat',
      model: 'test',
      status: 'completed',
      startedAt: _at,
      endedAt: '2026-10-09T12:01:00Z',
    ),
  ],
  messages: const [
    AgentMessage(
      id: 'u1',
      role: 'user',
      content: 'one',
      text: 'one',
      toolCalls: [],
      isError: false,
      createdAt: '2026-10-09T11:59:00Z',
    ),
    AgentMessage(
      id: 'a1',
      role: 'assistant',
      content: 'First answer',
      text: 'First answer',
      toolCalls: [AgentToolCall(id: 'c1', name: 'js', argumentsJson: '{}')],
      isError: false,
      createdAt: _at,
    ),
    AgentMessage(
      id: 't1',
      role: 'tool',
      content: '1',
      text: '1',
      toolCalls: [],
      toolCallId: 'c1',
      isError: false,
      createdAt: _at,
    ),
    AgentMessage(
      id: 'a2',
      role: 'assistant',
      content: 'Second answer',
      text: 'Second answer',
      toolCalls: [],
      isError: false,
      createdAt: _at,
    ),
    AgentMessage(
      id: 'a3',
      role: 'assistant',
      content: 'Third answer',
      text: 'Third answer',
      toolCalls: [AgentToolCall(id: 'c3', name: 'js', argumentsJson: '{}')],
      isError: false,
      createdAt: _at,
    ),
    AgentMessage(
      id: 't3',
      role: 'tool',
      content: '3',
      text: '3',
      toolCalls: [],
      toolCallId: 'c3',
      isError: false,
      createdAt: _at,
    ),
  ],
  activity: const [
    AgentToolActivity(
      runId: 'run',
      callId: 'c1',
      order: 1,
      name: 'js',
      summary: 'Work one',
      isError: false,
    ),
    AgentToolActivity(
      runId: 'run',
      callId: 'c3',
      order: 2,
      name: 'js',
      summary: 'Work three',
      isError: false,
    ),
  ],
  edits: const [],
);

void main() {
  test('without segments a run is one turn, as before', () {
    final anchors = <String, String>{};
    final items = agentTurns(_session(), anchorOf: anchors);
    expect(items.map((i) => i.id), ['u1', 'run']);
    final turn = items.whereType<AgentTurn>().single;
    expect(turn.texts.map((t) => t.text), [
      'First answer',
      'Second answer',
      'Third answer',
    ]);
    expect(turn.activity, hasLength(2));
    expect(
      {for (final e in anchors.entries) e.key: e.value},
      {
        'u1': 'u1',
        'a1': 'run',
        't1': 'run',
        'a2': 'run',
        'a3': 'run',
        't3': 'run',
      },
    );
  });

  test('a turn crossing segments splits into one piece per segment', () {
    final anchors = <String, String>{};
    final items = agentTurns(
      _session(),
      anchorOf: anchors,
      segmentOf: const {
        'u1': '-',
        'a1': '0.0',
        't1': '0.0',
        'a2': '1.0',
        'a3': '1.1',
        't3': '1.1',
      },
    );
    expect(items.map((i) => i.id), ['u1', 'run#0.0', 'run#1.0', 'run#1.1']);
    final pieces = items.whereType<AgentTurn>().toList();

    // Each piece holds only its own texts.
    expect(pieces[0].texts.map((t) => t.text), ['First answer']);
    expect(pieces[1].texts.map((t) => t.text), ['Second answer']);
    expect(pieces[2].texts.map((t) => t.text), ['Third answer']);

    // Work appears once, with its tool call's piece.
    expect(pieces[0].activity.map((a) => a.summary), ['Work one']);
    expect(pieces[1].activity, isEmpty);
    expect(pieces[2].activity.map((a) => a.summary), ['Work three']);
    expect(pieces.expand((p) => p.activity).length, _session().activity.length);
  });

  test('a message anchors to the piece that shows it, tool results too', () {
    final anchors = <String, String>{};
    agentTurns(
      _session(),
      anchorOf: anchors,
      segmentOf: const {
        'a1': '0.0',
        't1': '0.0',
        'a2': '1.0',
        'a3': '1.1',
        't3': '1.1',
      },
    );
    expect(anchors['a1'], 'run#0.0');
    expect(anchors['t1'], 'run#0.0');
    expect(anchors['a2'], 'run#1.0');
    expect(anchors['a3'], 'run#1.1');
    expect(anchors['t3'], 'run#1.1');
  });

  test(
    'a span across a boundary resolves to the piece of its first message',
    () {
      final anchors = <String, String>{};
      final items = agentTurns(
        _session(),
        anchorOf: anchors,
        segmentOf: const {
          'a1': '0.0',
          't1': '0.0',
          'a2': '1.0',
          'a3': '1.1',
          't3': '1.1',
        },
      );
      final outline = Outline([
        OutlineChapter(id: 'A', title: 'A', first: 'a1', last: 't1'),
        OutlineChapter(id: 'B', title: 'B', first: 'a2', last: 't3'),
      ]);
      final layout = OutlineLayout.build(
        outline: outline,
        itemIds: [for (final item in items) item.id],
        anchorIds: const ['u1', 'a1', 't1', 'a2', 'a3', 't3'],
        anchorOf: anchors,
        folded: (_) => false,
      );
      expect(layout.target('a2', 't3')?.itemId, 'run#1.0');
      expect(layout.target('a3', 'a3')?.itemId, 'run#1.1');
      expect(layout.chapterOfItem('run#0.0'), 0);
      expect(layout.chapterOfItem('run#1.1'), 1);
    },
  );

  test('items placed out of order never revisit a chapter', () {
    final outline = Outline([
      OutlineChapter(id: 'A', title: 'A', first: 'a1', last: 't1'),
      OutlineChapter(id: 'B', title: 'B', first: 'a2', last: 't3'),
    ]);
    // "late" shows B's messages but comes first; "early" shows A's, after it.
    const items = <ChatItem>[
      AssistantText('late', 'Late'),
      AssistantText('early', 'Early'),
    ];
    final layout = OutlineLayout.build(
      outline: outline,
      itemIds: [for (final item in items) item.id],
      anchorIds: const ['u1', 'a1', 't1', 'a2', 'a3', 't3'],
      anchorOf: const {
        'a2': 'late',
        't3': 'late',
        'a1': 'early',
        't1': 'early',
      },
      folded: (_) => false,
    );
    expect(
      layout.rows.where((r) => r.key.startsWith('chap:')).map((r) => r.key),
      ['chap:B'],
    );
    expect(layout.chapterOfItem('early'), 1);
  });
}
