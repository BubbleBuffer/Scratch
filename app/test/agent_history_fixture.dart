// A chat of 30 question and answer pairs (messages 0..59) with the compacted
// history the store would return for it:
//
//   0..6    legacy summary (no title)
//   6..24   chapter "Usage bar" (closed): episode 6..16 with actions, episode
//           16..24 summarized before actions existed
//   24..38  chapter "Reconnect" (closed, split): episode 24..30, 30..32 shown
//           in full, episode 32..38; only the second part has the counts
//   38..44  chapter "Settings" (open): one episode
//   44..60  live end; the boundary is 44, so "message 45"

import 'package:notes_app/src/rust/api/agent.dart';

const historyPairs = 30;

String historyStamp(int i) =>
    DateTime.utc(2026, 10, 27, 9).add(Duration(minutes: i)).toIso8601String();

AgentSessionState historyChat(String id) => AgentSessionState(
  session: AgentSession(
    id: id,
    title: 'Long chat',
    createdAt: '2026-10-27T09:00:00Z',
    updatedAt: '2026-10-27T09:00:00Z',
  ),
  seq: 0,
  liveSeen: 0,
  streamingText: '',
  pendingApprovals: const [],
  runs: const [],
  activity: const [],
  edits: const [],
  messages: [
    for (var i = 0; i < historyPairs; i++) ...[
      AgentMessage(
        id: 'u$i',
        role: 'user',
        content: 'Question $i',
        text: 'Question $i',
        toolCalls: const [],
        isError: false,
        createdAt: historyStamp(2 * i),
      ),
      AgentMessage(
        id: 'a$i',
        role: 'assistant',
        content: 'Answer $i',
        text: 'Answer $i',
        toolCalls: const [],
        isError: false,
        createdAt: historyStamp(2 * i + 1),
      ),
    ],
  ],
);

String historyMessageId(int index) =>
    '${index.isEven ? 'u' : 'a'}${index ~/ 2}';

List<String> historyMessageIds() => [
  for (var i = 0; i < historyPairs * 2; i++) historyMessageId(i),
];

MessageRange _r(int a, int b) => MessageRange(start: a, end: b);

EpisodeCard _episode(
  String id,
  int a,
  int b,
  String title, {
  String? gist,
  List<EpisodeAction> actions = const [],
}) => EpisodeCard(
  id: id,
  startedAt: historyStamp(a),
  endedAt: historyStamp(b - 1),
  firstMessageId: historyMessageId(a),
  lastMessageId: historyMessageId(b - 1),
  range: _r(a, b),
  messageCount: b - a,
  title: title,
  gist: gist,
  actions: actions,
  actionCount: actions.length,
  flaggedCount: actions.where((x) => x.pointOfInterest != null).length,
  errorCount: actions.where((x) => x.pointOfInterest?.kind == 'error').length,
);

EpisodeAction _action(
  String verb,
  String target, {
  TargetKind kind = TargetKind.path,
  String? marker,
  PointOfInterest? poi,
}) => EpisodeAction(
  verb: verb,
  target: target,
  targetKind: kind,
  changeMarker: marker,
  pointOfInterest: poi,
);

ChapterCard _chapter(
  String id,
  ChapterStatus status,
  String title,
  String? summary,
  int part,
  int parts,
  List<EpisodeCard> episodes, {
  int? messageCount,
  int? episodeCount,
}) => ChapterCard(
  id: id,
  status: status,
  title: title,
  summary: summary,
  part_: part,
  parts: parts,
  startedAt: episodes.first.startedAt,
  endedAt: episodes.last.endedAt,
  firstMessageId: episodes.first.firstMessageId,
  lastMessageId: episodes.last.lastMessageId,
  range: _r(episodes.first.range.start, episodes.last.range.end),
  messageCount: messageCount,
  episodeCount: episodeCount,
  episodes: episodes,
);

HistorySegment _card(ChapterCard c) => HistorySegment(
  kind: HistorySegmentKind.chapterCard,
  range: c.range,
  chapter: c,
);

HistorySegment _live(int a, int b) =>
    HistorySegment(kind: HistorySegmentKind.messages, range: _r(a, b));

/// The history of [historyChat]. [compactable] false is a chat the store
/// cannot summarize: no segments, boundary 0.
CompactedHistory historyOf({bool compactable = true}) {
  if (!compactable) {
    return CompactedHistory(
      compactable: false,
      boundary: 0,
      messageCount: historyPairs * 2,
      messageIds: historyMessageIds(),
      segments: const [],
    );
  }
  final log = [
    _action('Read', 'lib/usage_bar.dart'),
    _action('Edited', 'lib/usage_bar.dart', marker: '+12 −3'),
    _action(
      'Ran',
      'flutter test',
      kind: TargetKind.command,
      marker: 'failed · 2 tests',
      poi: const PointOfInterest(kind: 'error', reason: 'socket closes early'),
    ),
    _action('Edited', 'lib/usage_bar.dart', marker: '+4 −1'),
    _action(
      'Ran',
      'flutter test',
      kind: TargetKind.command,
      marker: 'passed · 14',
    ),
    _action('Created', 'test/usage_bar_test.dart', marker: 'created'),
    _action('Read', 'lib/socket.dart'),
    _action('Read', 'lib/backoff.dart'),
  ];
  final legacy = LegacyCard(
    id: 'lg1',
    kind: LegacyKind.checkpoint,
    summary: 'Set up the project and agreed on the usage bar behaviour.',
    startedAt: historyStamp(0),
    endedAt: historyStamp(5),
    firstMessageId: historyMessageId(0),
    lastMessageId: historyMessageId(5),
    range: _r(0, 6),
    messageCount: 6,
  );
  final usage = _chapter(
    'ch1',
    ChapterStatus.closed,
    'Usage bar',
    'The bar drops to zero when the socket closes. It now holds the last value.',
    1,
    1,
    [
      _episode(
        'ep1',
        6,
        16,
        'Hold the last value',
        gist: 'The bar keeps its figure on reconnect.',
        actions: log,
      ),
      _episode(
        'ep2',
        16,
        24,
        'Older summary',
        gist: 'Summarized before action logs.',
      ),
    ],
    messageCount: 18,
    episodeCount: 2,
  );
  final reconnect1 = _chapter(
    'ch2',
    ChapterStatus.closed,
    'Reconnect',
    'Backoff and retry for the usage socket.',
    1,
    2,
    [_episode('ep3', 24, 30, 'Backoff', gist: 'Retry with growing delay.')],
  );
  final reconnect2 = _chapter(
    'ch2',
    ChapterStatus.closed,
    'Reconnect',
    'Backoff and retry for the usage socket.',
    2,
    2,
    [_episode('ep4', 32, 38, 'Retry cap', gist: 'Stop after five tries.')],
    messageCount: 12,
    episodeCount: 2,
  );
  final settings = _chapter(
    'ch3',
    ChapterStatus.open,
    'Settings',
    null,
    1,
    1,
    [
      _episode(
        'ep5',
        38,
        44,
        'Chat history setting',
        gist: 'A per-chat choice.',
        actions: [_action('Read', 'lib/settings.dart')],
      ),
    ],
    messageCount: 6,
    episodeCount: 1,
  );
  return CompactedHistory(
    compactable: true,
    boundary: 44,
    messageCount: historyPairs * 2,
    messageIds: historyMessageIds(),
    segments: [
      HistorySegment(
        kind: HistorySegmentKind.legacyCard,
        range: legacy.range,
        legacy: legacy,
      ),
      _card(usage),
      _card(reconnect1),
      _live(30, 32),
      _card(reconnect2),
      _card(settings),
      _live(44, historyPairs * 2),
    ],
  );
}
