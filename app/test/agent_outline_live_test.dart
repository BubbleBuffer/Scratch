// The chat outline as it changes while it is on screen: an outline arriving on
// a plain chat keeps the reader's place, a chapter that closes under the reader
// stays open until they scroll away or tap Fold, a fold collapses its messages
// over a few frames, an outline upsert reloads the outline on its revision
// alone, and a chapter's fold state follows its first message across id
// changes.

import 'package:flutter/widgets.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:notes_app/addons/agent/api.dart';
import 'package:notes_app/shell/app_shell.dart';
import 'package:notes_app/src/rust/api/agent.dart';
import 'package:notes_app/state/app_state.dart';

import 'agent_test.dart' show openAgentStore;
import 'harness.dart';
import 'sync_helpers.dart';

Finder key(String k) => find.byKey(ValueKey(k));

const _chapters = 3;
const _pairs = 40;
const _half = _pairs ~/ 2;

String _stamp(int minute) => DateTime.utc(
  2026,
  10,
  9,
  12,
).add(Duration(minutes: minute)).toIso8601String();

/// A message's text. The number is padded to three digits, so each text is
/// unique under [find.textContaining], which matches substrings.
String _text(String word, int i) => '$word ${i.toString().padLeft(3, '0')}';

// Three chapters of 40 question and answer pairs, each with two sections.
AgentSessionState _chat(String id) => AgentSessionState(
  session: AgentSession(
    id: id,
    title: 'Live chat',
    createdAt: '2026-10-09T12:00:00Z',
    updatedAt: '2026-10-09T12:00:00Z',
  ),
  seq: 0,
  liveSeen: 0,
  streamingText: '',
  pendingApprovals: const [],
  runs: const [],
  activity: const [],
  edits: const [],
  messages: [
    for (var i = 0; i < _chapters * _pairs; i++) ...[
      AgentMessage(
        id: 'u$i',
        role: 'user',
        content: _text('Question', i),
        text: _text('Question', i),
        toolCalls: const [],
        isError: false,
        createdAt: _stamp(2 * i),
      ),
      AgentMessage(
        id: 'a$i',
        role: 'assistant',
        content: _text('Answer', i),
        text: _text('Answer', i),
        toolCalls: const [],
        isError: false,
        createdAt: _stamp(2 * i + 1),
      ),
    ],
  ],
);

/// Three chapters. Chapter 0 is open when [open0], otherwise closed under the
/// id [id0]. Chapters 1 and 2 are always closed.
ChatOutline _outline({bool open0 = false, String id0 = 'c0'}) => ChatOutline(
  chapters: [
    for (var c = 0; c < _chapters; c++)
      OutlineChapter(
        id: c == 0 ? id0 : 'c$c',
        title: 'Chapter $c',
        summary: 'Summary for chapter $c. More detail follows.',
        decisions: ['Decision $c'],
        ongoing: c == 0 && open0,
        firstMessageId: 'u${c * _pairs}',
        lastMessageId: 'a${(c + 1) * _pairs - 1}',
        startedAt: _stamp(c * _pairs * 2),
        endedAt: _stamp((c + 1) * _pairs * 2 - 1),
        sections: [
          for (var s = 0; s < 2; s++)
            OutlineSection(
              id: 'c${c}s$s',
              title: 'Section $c.$s',
              summary: '',
              decisions: const [],
              firstMessageId: 'u${c * _pairs + s * _half}',
              lastMessageId: 'a${c * _pairs + (s + 1) * _half - 1}',
            ),
        ],
      ),
  ],
);

Future<void> _pumpChat(
  WidgetTester tester,
  AgentStore store, {
  bool outlined = true,
}) async {
  await tester.pumpWidget(
    NotesShell(
      forceDark: false,
      navigatorKey: GlobalKey<NavigatorState>(),
      wrap: (child) => AppScope(state: current.app, child: child),
      home: Center(
        child: SizedBox(
          width: 480,
          height: 650,
          child: AgentChat(store: store),
        ),
      ),
    ),
  );
  await settle(tester);
  store.seedFixture(
    _chat(store.openId!),
    outlined ? _outline() : ChatOutline(chapters: const []),
  );
  await settle(tester);
}

/// Scrolls the transcript by [dy] pixels (negative moves towards the end).
Future<void> _scroll(WidgetTester tester, double dy) async {
  await tester.drag(_transcript(), Offset(0, dy));
  await settle(tester, min: const Duration(milliseconds: 300));
}

Finder _transcript() => find
    .descendant(
      of: find.byType(AgentChat),
      matching: find.byWidgetPredicate(
        (w) => w is CustomScrollView || w is ListView,
      ),
    )
    .first;

/// The top edge of the first built widget showing [text], or null.
double? _topOf(WidgetTester tester, String text) {
  final f = find.textContaining(text);
  return f.evaluate().isEmpty ? null : tester.getTopLeft(f.first).dy;
}

/// The text of the message whose top edge is nearest the top of the transcript.
String _nearestToTop(WidgetTester tester) {
  final top = tester.getTopLeft(_transcript()).dy;
  String? best;
  var bestDistance = double.infinity;
  for (var i = 0; i < _chapters * _pairs; i++) {
    for (final text in [_text('Question', i), _text('Answer', i)]) {
      final y = _topOf(tester, text);
      // A message that starts above the viewport is only partly read; the
      // contract is about the first message the reader sees from its start.
      if (y == null || y < top) continue;
      final distance = (y - top).abs();
      if (distance < bestDistance) {
        bestDistance = distance;
        best = text;
      }
    }
  }
  return best!;
}

/// Scrolls the transcript until chapter [id]'s heading is on screen. A test
/// makes this jump itself, before it taps Fold on the heading.
Future<void> _revealHeading(WidgetTester tester, String id) async {
  await tapAndSettle(tester, key('outline-where'));
  await tapAndSettle(tester, key('outline-contents-chapter-$id'));
  await settle(tester, min: const Duration(milliseconds: 300));
}

Finder _fold() => find.textContaining('Fold');

void main() {
  setUpAll(initCore);

  appTest('an outline arriving on a plain chat keeps the reader in place', (
    tester,
  ) async {
    await pumpApp(tester, mobile: false, empty: true);
    final store = await openAgentStore(tester);
    await real(tester, () => store.createSession(title: 'Live chat'));
    await _pumpChat(tester, store, outlined: false);
    await _scroll(tester, 5000);

    final text = _nearestToTop(tester);
    final before = _topOf(tester, text)!;

    store.seedFixture(_chat(store.openId!), _outline(open0: true));
    await settle(tester, min: const Duration(milliseconds: 300));

    // The position contract: the same message stays at the same height.
    expect(_topOf(tester, text), closeTo(before, 1));
    expect(tester.takeException(), isNull);
  });

  appTest(
    'messages outside every chapter keep their rows and the chapters load',
    (tester) async {
      await pumpApp(tester, mobile: false, empty: true);
      final store = await openAgentStore(tester);
      await real(tester, () => store.createSession(title: 'Live chat'));
      await _pumpChat(tester, store, outlined: false);

      // Chapters 1 and 2 only: the first chapter's messages sit before every
      // chapter, and a second outline with chapter 0 alone leaves the rest after
      // the last chapter.
      store.seedFixture(
        _chat(store.openId!),
        ChatOutline(chapters: _outline().chapters.skip(1).toList()),
      );
      await settle(tester);
      await _revealHeading(tester, 'c1');
      expect(key('outline-chapter-c1'), findsOneWidget);
      expect(tester.takeException(), isNull);

      store.seedFixture(
        _chat(store.openId!),
        ChatOutline(chapters: _outline().chapters.take(1).toList()),
      );
      await settle(tester);
      await _revealHeading(tester, 'c0');
      expect(key('outline-chapter-c0'), findsOneWidget);
      expect(tester.takeException(), isNull);
    },
  );

  appTest('a chapter that closes under the reader stays open until Fold', (
    tester,
  ) async {
    await pumpApp(tester, mobile: false, empty: true);
    final store = await openAgentStore(tester);
    await real(tester, () => store.createSession(title: 'Live chat'));
    await _pumpChat(tester, store);
    store.seedFixture(_chat(store.openId!), _outline(open0: true));
    await settle(tester);
    await _scroll(tester, 5000);

    final text = _nearestToTop(tester);
    final before = _topOf(tester, text)!;
    store.seedFixture(_chat(store.openId!), _outline());
    await settle(tester, min: const Duration(milliseconds: 600));

    expect(find.textContaining('Chapter closed'), findsOneWidget);
    expect(find.textContaining(_text('Answer', 0)), findsWidgets);
    // Closing alone does not move the reader.
    expect(_topOf(tester, text), closeTo(before, 2));

    await _revealHeading(tester, 'c0');
    await tapAndSettle(tester, _fold());
    await settle(tester, min: const Duration(milliseconds: 800));

    expect(find.textContaining('Chapter closed'), findsNothing);
    expect(find.textContaining(_text('Answer', 0)), findsNothing);
    expect(key('outline-chapter-c0'), findsOneWidget);
    expect(tester.takeException(), isNull);
  });

  appTest(
    'a chapter extending to the visible message and closing together defers its fold',
    (tester) async {
      await pumpApp(tester, mobile: false, empty: true);
      final store = await openAgentStore(tester);
      await real(tester, () => store.createSession(title: 'Growing chat'));
      await _pumpChat(tester, store);
      final full = _outline(open0: true);
      final first = full.chapters.first;
      store.seedFixture(
        _chat(store.openId!),
        ChatOutline(
          chapters: [
            OutlineChapter(
              id: first.id,
              title: first.title,
              summary: first.summary,
              decisions: first.decisions,
              ongoing: true,
              firstMessageId: first.firstMessageId,
              lastMessageId: 'a9',
              startedAt: first.startedAt,
              endedAt: null,
              sections: const [],
            ),
            ...full.chapters.skip(1),
          ],
        ),
      );
      await settle(tester);
      await _scroll(tester, -3000);
      final text = _nearestToTop(tester);
      final before = _topOf(tester, text)!;
      store.seedFixture(_chat(store.openId!), _outline());
      await settle(tester, min: const Duration(milliseconds: 600));
      expect(_topOf(tester, text), closeTo(before, 2));
      await _revealHeading(tester, 'c0');
      expect(find.textContaining('Chapter closed'), findsOneWidget);
      expect(tester.takeException(), isNull);
    },
  );

  appTest('a deferred chapter folds once the reader scrolls away', (
    tester,
  ) async {
    await pumpApp(tester, mobile: false, empty: true);
    final store = await openAgentStore(tester);
    await real(tester, () => store.createSession(title: 'Live chat'));
    await _pumpChat(tester, store);
    store.seedFixture(_chat(store.openId!), _outline(open0: true));
    await settle(tester);
    await _scroll(tester, 5000);

    store.seedFixture(_chat(store.openId!), _outline());
    await settle(tester, min: const Duration(milliseconds: 600));
    expect(find.textContaining('Chapter closed'), findsOneWidget);

    await _revealHeading(tester, 'c1');
    await settle(tester, min: const Duration(milliseconds: 800));

    expect(find.textContaining('Chapter closed'), findsNothing);
    expect(find.textContaining(_text('Answer', 0)), findsNothing);
    await _revealHeading(tester, 'c0');
    expect(key('outline-chapter-c0'), findsOneWidget);
    expect(tester.takeException(), isNull);
  });

  appTest('a fold collapses its messages over frames before hiding them', (
    tester,
  ) async {
    await pumpApp(tester, mobile: false, empty: true);
    final store = await openAgentStore(tester);
    await real(tester, () => store.createSession(title: 'Live chat'));
    await _pumpChat(tester, store);
    store.seedFixture(_chat(store.openId!), _outline(open0: true));
    await settle(tester);
    await _scroll(tester, 5000);

    store.seedFixture(_chat(store.openId!), _outline());
    await settle(tester, min: const Duration(milliseconds: 600));
    expect(find.textContaining('Chapter closed'), findsOneWidget);

    await _revealHeading(tester, 'c0');
    await tester.tap(_fold());
    await tester.pump();
    await tester.pump();
    await tester.pump(const Duration(milliseconds: 120));

    // Mid-collapse: the messages are still built, inside a size transition
    // part way between open and closed.
    final answer = find.textContaining(_text('Answer', 0));
    expect(answer, findsWidgets);
    final shrinking = tester
        .widgetList<SizeTransition>(
          find.ancestor(of: answer, matching: find.byType(SizeTransition)),
        )
        .toList();
    expect(shrinking, isNotEmpty);
    expect(
      shrinking.map((t) => t.sizeFactor.value),
      everyElement(allOf(greaterThan(0), lessThan(1))),
    );

    await settle(tester, min: const Duration(milliseconds: 800));

    expect(find.textContaining('Chapter closed'), findsNothing);
    expect(find.textContaining(_text('Answer', 0)), findsNothing);
    expect(key('outline-chapter-c0'), findsOneWidget);
    expect(tester.takeException(), isNull);
  });

  appTest('an outline upsert with no message events reloads the outline', (
    tester,
  ) async {
    await pumpApp(tester, mobile: false, empty: true);
    final store = await openAgentStore(tester);
    await real(tester, () => store.createSession(title: 'Live chat'));
    await _pumpChat(tester, store, outlined: false);

    var revision = BigInt.zero;
    var polls = 0;
    AgentStore.debugRevisionLoader = (agent, id) async {
      polls++;
      return revision;
    };
    addTearDown(() => AgentStore.debugRevisionLoader = null);

    store.seedFixture(_chat(store.openId!), _outline());
    final seeded = store.outline;
    await settle(tester, min: const Duration(milliseconds: 1200));

    // Polling runs, and an unchanged revision reloads nothing.
    expect(polls, greaterThan(0));
    expect(store.outline, same(seeded));

    // The same messages under a new revision: only the outline changed, and
    // the poll reloads it.
    revision = BigInt.one;
    await settle(tester, min: const Duration(milliseconds: 1200));

    expect(store.outline, isNot(same(seeded)));
    expect(tester.takeException(), isNull);
  });

  appTest(
    'a folded-by-hand chapter keeps its state across chapter id changes',
    (tester) async {
      await pumpApp(tester, mobile: false, empty: true);
      final store = await openAgentStore(tester);
      await real(tester, () => store.createSession(title: 'Live chat'));
      await _pumpChat(tester, store);

      expect(find.textContaining(_text('Answer', 0)), findsNothing);
      await tapAndSettle(tester, key('outline-card-show-c0'));
      expect(find.textContaining(_text('Answer', 0)), findsWidgets);

      store.seedFixture(_chat(store.openId!), _outline(id0: 'c0-final'));
      await settle(tester);

      expect(key('outline-chapter-c0-final'), findsOneWidget);
      expect(key('outline-chapter-c0'), findsNothing);
      expect(find.text('Hide messages'), findsOneWidget);
      expect(find.textContaining(_text('Answer', 0)), findsWidgets);
      expect(tester.takeException(), isNull);
    },
  );

  appTest(
    'a new section above the reader preserves the visible message position',
    (tester) async {
      await pumpApp(tester, mobile: false, empty: true);
      final store = await openAgentStore(tester);
      await real(tester, () => store.createSession(title: 'Live chat'));
      await _pumpChat(tester, store);
      store.seedFixture(_chat(store.openId!), _outline(open0: true));
      await settle(tester);
      await tapAndSettle(tester, key('outline-where'));
      await tapAndSettle(tester, key('outline-contents-section-c0s1'));
      await _scroll(tester, -180);
      final text = _nearestToTop(tester);
      final before = _topOf(tester, text)!;
      final previous = _outline(open0: true);
      final chapter = previous.chapters.first;
      final updated = OutlineChapter(
        id: chapter.id,
        title: chapter.title,
        summary: chapter.summary,
        decisions: chapter.decisions,
        ongoing: chapter.ongoing,
        firstMessageId: chapter.firstMessageId,
        lastMessageId: chapter.lastMessageId,
        startedAt: chapter.startedAt,
        endedAt: chapter.endedAt,
        sections: [
          OutlineSection(
            id: 'new-section',
            title: 'A newly completed section',
            summary: '',
            decisions: const [],
            firstMessageId: 'u0',
            lastMessageId: 'a9',
          ),
          OutlineSection(
            id: 'c0s0',
            title: 'Section 0.0',
            summary: '',
            decisions: const [],
            firstMessageId: 'u10',
            lastMessageId: 'a19',
          ),
          chapter.sections.last,
        ],
      );
      store.seedFixture(
        _chat(store.openId!),
        ChatOutline(chapters: [updated, ...previous.chapters.skip(1)]),
      );
      await settle(tester, min: const Duration(milliseconds: 300));
      expect(_topOf(tester, text), closeTo(before, 1));
      await tapAndSettle(tester, key('outline-where'));
      expect(find.text('A newly completed section'), findsOneWidget);
      expect(tester.takeException(), isNull);
    },
  );

  appTest('a card keeps its expanded details across chapter ID changes', (
    tester,
  ) async {
    await pumpApp(tester, mobile: false, empty: true);
    final store = await openAgentStore(tester);
    await real(tester, () => store.createSession(title: 'Live chat'));
    await _pumpChat(tester, store);
    await tapAndSettle(tester, key('outline-card-face-c0'));
    expect(
      find.text('Summary for chapter 0. More detail follows.'),
      findsOneWidget,
    );
    store.seedFixture(_chat(store.openId!), _outline(id0: 'c0-final'));
    await settle(tester);
    expect(key('outline-card-face-c0-final'), findsOneWidget);
    expect(
      find.text('Summary for chapter 0. More detail follows.'),
      findsOneWidget,
    );
    expect(tester.takeException(), isNull);
  });

  appTest(
    'splitting the grouped turn being read preserves its message position',
    (tester) async {
      await pumpApp(tester, mobile: false, empty: true);
      final store = await openAgentStore(tester);
      await real(tester, () => store.createSession(title: 'Live chat'));
      await _pumpChat(tester, store);
      final base = _chat(store.openId!);
      final grouped = AgentSessionState(
        session: base.session,
        seq: base.seq,
        liveSeen: base.liveSeen,
        streamingText: '',
        pendingApprovals: const [],
        activity: const [],
        edits: const [],
        runs: [
          AgentRunRecord(
            id: 'one-run',
            sessionId: store.openId!,
            model: 'test',
            status: 'completed',
            startedAt: _stamp(1),
            endedAt: _stamp(240),
          ),
        ],
        messages: [
          base.messages.first,
          ...base.messages.where((m) => m.role == 'assistant'),
        ],
      );
      ChatOutline outline(bool split) => ChatOutline(
        chapters: [
          OutlineChapter(
            id: 'grouped-chapter',
            title: 'Grouped chapter',
            summary: '',
            decisions: const [],
            ongoing: true,
            firstMessageId: 'u0',
            lastMessageId: 'a119',
            startedAt: _stamp(0),
            endedAt: null,
            sections: [
              OutlineSection(
                id: 'first-half',
                title: 'First half',
                summary: '',
                decisions: const [],
                firstMessageId: 'a0',
                lastMessageId: 'a59',
              ),
              OutlineSection(
                id: 'second-half',
                title: 'Second half',
                summary: '',
                decisions: const [],
                firstMessageId: 'a60',
                lastMessageId: split ? 'a79' : 'a119',
              ),
              if (split)
                OutlineSection(
                  id: 'new-tail',
                  title: 'New tail',
                  summary: '',
                  decisions: const [],
                  firstMessageId: 'a80',
                  lastMessageId: 'a119',
                ),
            ],
          ),
        ],
      );
      store.seedFixture(grouped, outline(false));
      await settle(tester);
      await tapAndSettle(tester, key('outline-where'));
      await tapAndSettle(tester, key('outline-contents-section-second-half'));
      await _scroll(tester, -780);
      final text = _nearestToTop(tester);
      final before = _topOf(tester, text)!;
      store.seedFixture(grouped, outline(true));
      await settle(tester, min: const Duration(milliseconds: 300));
      expect(_topOf(tester, text), closeTo(before, 1));
      expect(tester.takeException(), isNull);
    },
  );
}
