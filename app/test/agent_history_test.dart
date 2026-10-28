// Compacted history in the chat, on the real harness: the chat opens on the
// "Full messages from here" hairline, a closed chapter opens into episode
// cards and from there into the original messages, switching the setting
// keeps the draft and the place in the chat, and a chat the store cannot
// summarize reads as Full. Goldens are in agent_history_golden_test.dart.

import 'package:flutter/widgets.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:notes_app/addons/agent/api.dart';
import 'package:notes_app/shell/app_shell.dart';
import 'package:notes_app/src/rust/api/agent.dart' show ChatOutline;
import 'package:notes_app/state/app_state.dart';

import 'agent_history_fixture.dart';
import 'agent_test.dart' show openAgentStore;
import 'harness.dart';
import 'sync_helpers.dart';

const _list = 'agent-history-list';

/// Creates the real session the fixture belongs to and opens it in [store].
Future<void> _seed(WidgetTester tester, AgentStore store) async {
  await real(tester, () => store.createSession(title: 'Long chat'));
}

/// Mounts the chat and installs the fixture once the mount has settled (the
/// mount's view sync would otherwise replace it).
Future<void> pumpHistoryChat(
  WidgetTester tester,
  AgentStore store, {
  bool compacted = true,
  bool compactable = true,
  bool dark = false,
  double width = 560,
  double height = 700,
}) async {
  await tester.pumpWidget(
    NotesShell(
      forceDark: dark,
      navigatorKey: GlobalKey<NavigatorState>(),
      wrap: (child) => AppScope(state: current.app, child: child),
      home: Center(
        child: RepaintBoundary(
          key: const ValueKey('history-shot'),
          child: SizedBox(
            width: width,
            height: height,
            child: AgentChat(store: store),
          ),
        ),
      ),
    ),
  );
  await settle(tester);
  final id = store.openId!;
  store.seedFixture(
    historyChat(id),
    ChatOutline(chapters: const []),
    history: historyOf(compactable: compactable),
  );
  if (compacted) store.setHistoryCompacted(id, true);
  await settle(tester);
}

/// Drags the compacted list down (the chat up) until [target] is built.
Future<void> scrollUpTo(WidgetTester tester, Finder target) async {
  for (var i = 0; i < 40 && target.evaluate().isEmpty; i++) {
    await tester.drag(key(_list), const Offset(0, 300));
    await tester.pump(const Duration(milliseconds: 50));
  }
  await settle(tester);
  expect(target, findsOneWidget);
}

Future<void> tapVisible(WidgetTester tester, Finder target) async {
  await tester.ensureVisible(target);
  await settle(tester);
  await tapAndSettle(tester, target);
}

double _top(WidgetTester tester, String text) =>
    tester.getTopLeft(find.textContaining(text)).dy;

void main() {
  setUpAll(initCore);

  appTest('compacted opens on the hairline, with only the live end below', (
    tester,
  ) async {
    await pumpHistoryChat(tester, await _openedStore(tester));
    expect(key(_list), findsOneWidget);
    expect(find.text('Full messages from here'), findsOneWidget);
    expect(find.text('message 45'), findsOneWidget);
    expect(find.textContaining('Question 29'), findsOneWidget);
    // Older history is summaries, not messages.
    expect(find.textContaining('Question 3'), findsNothing);
    expect(find.textContaining('Question 10'), findsNothing);
    expect(tester.takeException(), isNull);
  });

  appTest('a closed chapter opens into episodes, and an episode into its '
      'action log and its original messages', (tester) async {
    await pumpHistoryChat(tester, await _openedStore(tester));
    await scrollUpTo(tester, key('history-chapter-ch1-toggle'));
    expect(find.text('Usage bar'), findsOneWidget);
    expect(find.text('Hold the last value'), findsNothing);

    await tapVisible(tester, key('history-chapter-ch1-toggle'));
    expect(find.text('Hold the last value'), findsOneWidget);
    expect(find.text('Older summary'), findsOneWidget);

    // The episode with actions opens its log, and +N more folds past six.
    await tapVisible(tester, key('history-episode-ep1-header'));
    expect(key('history-episode-ep1-log-rows'), findsOneWidget);
    expect(key('history-episode-ep1-more'), findsOneWidget);

    // The summary-only episode goes straight to the originals.
    await tapVisible(tester, key('history-episode-ep2-messages'));
    expect(find.textContaining('Question 8'), findsOneWidget);
    expect(find.textContaining('Answer 11'), findsOneWidget);
    await tapVisible(tester, key('history-chapter-ch1-toggle'));
    expect(find.text('Hold the last value'), findsNothing);
    expect(tester.takeException(), isNull);
  });

  appTest('switching the setting keeps the draft and the place in the chat', (
    tester,
  ) async {
    final store = await _openedStore(tester);
    await pumpHistoryChat(tester, store);
    await tester.enterText(find.byType(EditableText).last, 'half a reply');
    await tester.pump();
    // Scroll the live end a little, then let the reader measure.
    await tester.drag(key(_list), const Offset(0, -120));
    await tester.pump(const Duration(milliseconds: 100));
    await settle(tester);
    final before = _top(tester, 'Question 25');

    await tapAndSettle(tester, key('chat-settings'));
    await tapAndSettle(tester, key('chat-history-full'));
    await tester.tapAt(const Offset(2, 2));
    await settle(tester);
    expect(key(_list), findsNothing);
    expect(find.text('Full messages from here'), findsNothing);
    expect(_top(tester, 'Question 25'), closeTo(before, 4));
    expect(
      tester
          .widget<EditableText>(find.byType(EditableText).last)
          .controller
          .text,
      'half a reply',
    );

    // And back: the cards return, the place and the draft are kept.
    await tester.pump(const Duration(milliseconds: 100));
    await settle(tester);
    final full = _top(tester, 'Question 25');
    await tapAndSettle(tester, key('chat-settings'));
    await tapAndSettle(tester, key('chat-history-compacted'));
    await tester.tapAt(const Offset(2, 2));
    await settle(tester);
    expect(key(_list), findsOneWidget);
    expect(_top(tester, 'Question 25'), closeTo(full, 4));
    expect(
      tester
          .widget<EditableText>(find.byType(EditableText).last)
          .controller
          .text,
      'half a reply',
    );
    expect(store.historyCompacted(store.openId), isTrue);
    expect(tester.takeException(), isNull);
  });

  appTest('Full is the default and shows no cards, with the setting offered', (
    tester,
  ) async {
    await pumpHistoryChat(tester, await _openedStore(tester), compacted: false);
    expect(key(_list), findsNothing);
    expect(key('chat-settings'), findsOneWidget);
    expect(find.text('Full messages from here'), findsNothing);
    expect(find.textContaining('Question 29'), findsOneWidget);
    await tapAndSettle(tester, key('chat-settings'));
    expect(find.text('Chat history'), findsOneWidget);
    expect(find.text('Chat settings'), findsOneWidget);
    expect(find.textContaining('Nothing is deleted.'), findsOneWidget);
    expect(tester.takeException(), isNull);
  });

  appTest('a chat the store cannot summarize reads as Full, with no notice', (
    tester,
  ) async {
    await pumpHistoryChat(
      tester,
      await _openedStore(tester),
      compactable: false,
    );
    expect(key(_list), findsNothing);
    expect(key('chat-settings'), findsNothing);
    expect(find.text('Full messages from here'), findsNothing);
    expect(find.textContaining('Question 29'), findsOneWidget);
    // The older messages are written out as ordinary rows.
    await tester.scrollUntilVisible(
      find.textContaining('Question 0'),
      -400,
      scrollable: find.byType(Scrollable).first,
    );
    expect(find.textContaining('Question 0'), findsWidgets);
    expect(tester.takeException(), isNull);
  });
}

Future<AgentStore> _openedStore(WidgetTester tester) async {
  await pumpApp(tester, mobile: false, empty: true);
  final store = await openAgentStore(tester);
  await _seed(tester, store);
  return store;
}
