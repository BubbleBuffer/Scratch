// The chat outline on the real harness: folding a chapter's messages in and
// out, the where-am-I strip, and the contents reaching far chapters and
// sections. Goldens for the folded card and the contents, in both themes.
// Baselines live under goldens/outline and are made with --update-goldens.

import 'package:agent_kit/agent_kit.dart' show AnchoredList, OutlineFlash;
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

// Three closed chapters of 40 question and answer pairs each: chapter c holds
// messages u(40c)..a(40c+39), and each chapter has two sections of 20 pairs.
AgentSessionState _longChat(String id) => AgentSessionState(
  session: AgentSession(
    id: id,
    title: 'Long chat',
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
        content: 'Question $i',
        text: 'Question $i',
        toolCalls: const [],
        isError: false,
        createdAt: _stamp(2 * i),
      ),
      AgentMessage(
        id: 'a$i',
        role: 'assistant',
        content: 'Answer $i',
        text: 'Answer $i',
        toolCalls: const [],
        isError: false,
        createdAt: _stamp(2 * i + 1),
      ),
    ],
  ],
);

ChatOutline _outline() => ChatOutline(
  chapters: [
    for (var c = 0; c < _chapters; c++)
      OutlineChapter(
        id: 'c$c',
        title: 'Chapter $c',
        summary: 'Summary for chapter $c. More detail follows.',
        decisions: ['Decision $c'],
        ongoing: false,
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

/// Creates the real session the fixture belongs to and opens it in [store].
/// The session is in the database, so the native view subscription and the
/// queries find it. The fixture's messages and chapters are installed by
/// [_pumpChat] once the chat has mounted: the mount's view sync reloads the
/// database's (empty) session, which would otherwise replace them.
Future<void> _seed(WidgetTester tester, AgentStore store) async {
  await real(tester, () => store.createSession(title: 'Long chat'));
}

/// Installs the fixture on the open chat of [store]. [outlined] false leaves
/// the chat without chapters: the plain transcript.
void _install(AgentStore store, {bool outlined = true}) {
  store.seedFixture(
    _longChat(store.openId!),
    outlined ? _outline() : ChatOutline(chapters: const []),
  );
}

Future<void> _pumpChat(
  WidgetTester tester,
  AgentStore store, {
  bool dark = false,
  bool panel = false,
  bool outlined = true,
}) async {
  await tester.pumpWidget(
    NotesShell(
      forceDark: dark,
      navigatorKey: GlobalKey<NavigatorState>(),
      wrap: (child) => AppScope(state: current.app, child: child),
      home: Center(
        child: SizedBox(
          width: 480,
          height: 650,
          child: AgentChat(store: store, panel: panel),
        ),
      ),
    ),
  );
  await settle(tester);
  _install(store, outlined: outlined);
  await settle(tester);
}

/// The dock and the full Agent screen mounted together on one [store], each
/// under its own key. Both fit the 800 px test view.
Future<void> _pumpDockAndScreen(
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
        child: Row(
          mainAxisSize: MainAxisSize.min,
          crossAxisAlignment: CrossAxisAlignment.stretch,
          children: [
            SizedBox(
              width: 380,
              height: 650,
              child: KeyedSubtree(
                key: const ValueKey('dock-chat'),
                child: AgentChat(store: store, panel: true),
              ),
            ),
            SizedBox(
              width: 380,
              height: 650,
              child: KeyedSubtree(
                key: const ValueKey('screen-chat'),
                child: AgentChat(store: store),
              ),
            ),
          ],
        ),
      ),
    ),
  );
  await settle(tester);
  _install(store, outlined: outlined);
  await settle(tester);
}

Finder _strip(String text) => find.descendant(
  of: key('outline-where'),
  matching: find.textContaining(text),
);

/// The tint a flashing row paints: its wrapper's own box, the first one.
Color _tint(WidgetTester tester, Finder flash) {
  final box = tester.widget<DecoratedBox>(
    find.descendant(of: flash, matching: find.byType(DecoratedBox)).first,
  );
  return (box.decoration as BoxDecoration).color!;
}

void main() {
  setUpAll(initCore);

  appTest('a closed chapter shows as a card, and its messages open under it', (
    tester,
  ) async {
    await pumpApp(tester, mobile: false, empty: true);
    final store = await openAgentStore(tester);
    await _seed(tester, store);
    await _pumpChat(tester, store);

    expect(key('outline-chapter-c0'), findsOneWidget);
    expect(find.textContaining('Answer 0'), findsNothing);

    await tapAndSettle(tester, key('outline-card-show-c0'));
    expect(find.text('Hide messages'), findsOneWidget);
    expect(find.textContaining('Answer 0'), findsWidgets);

    await tapAndSettle(tester, key('outline-card-show-c0'));
    expect(find.textContaining('Answer 0'), findsNothing);
    expect(tester.takeException(), isNull);
  });

  appTest('the where strip names the chapter the contents jumped to', (
    tester,
  ) async {
    await pumpApp(tester, mobile: false, empty: true);
    final store = await openAgentStore(tester);
    await _seed(tester, store);
    await _pumpChat(tester, store);

    await tapAndSettle(tester, key('outline-where'));
    await tapAndSettle(tester, key('outline-contents-chapter-c0'));
    expect(_strip('Chapter 0'), findsOneWidget);
    expect(tester.takeException(), isNull);
  });

  appTest('a far contents chapter unfolds and shows its messages', (
    tester,
  ) async {
    await pumpApp(tester, mobile: false, empty: true);
    final store = await openAgentStore(tester);
    await _seed(tester, store);
    await _pumpChat(tester, store);

    await tapAndSettle(tester, key('outline-where'));
    await tapAndSettle(tester, key('outline-contents-chapter-c2'));
    expect(_strip('Chapter 2'), findsOneWidget);
    expect(find.textContaining('Answer 80'), findsWidgets);
    // Chapter 0 stays folded: its messages are not built.
    expect(find.textContaining('Answer 0'), findsNothing);
    expect(tester.takeException(), isNull);
  });

  appTest('the dock shares the strip and indexed Contents jumps', (
    tester,
  ) async {
    await pumpApp(tester, mobile: false, empty: true);
    final store = await openAgentStore(tester);
    await _seed(tester, store);
    await _pumpChat(tester, store, panel: true);
    await tapAndSettle(tester, key('outline-where'));
    await tapAndSettle(tester, key('outline-contents-section-c1s1'));
    expect(key('outline-section-sec:c1s1'), findsOneWidget);
    expect(_strip('Section 1.1'), findsOneWidget);
    expect(tester.takeException(), isNull);
  });

  appTest('a contents section jump reveals its divider and names it', (
    tester,
  ) async {
    await pumpApp(tester, mobile: false, empty: true);
    final store = await openAgentStore(tester);
    await _seed(tester, store);
    await _pumpChat(tester, store);

    await tapAndSettle(tester, key('outline-where'));
    await tapAndSettle(tester, key('outline-contents-section-c1s1'));
    expect(key('outline-section-sec:c1s1'), findsOneWidget);
    expect(_strip('Section 1.1'), findsOneWidget);
    expect(tester.takeException(), isNull);
  });

  appTest('the contents fit a phone: the list scrolls inside the sheet', (
    tester,
  ) async {
    tester.view.physicalSize = const Size(390, 780);
    tester.view.devicePixelRatio = 1;
    addTearDown(tester.view.resetPhysicalSize);
    addTearDown(tester.view.resetDevicePixelRatio);
    await pumpApp(tester, mobile: false, empty: true);
    final store = await openAgentStore(tester);
    await _seed(tester, store);
    await _pumpChat(tester, store);

    await tapAndSettle(tester, key('outline-where'));
    expect(key('outline-contents-scroll'), findsOneWidget);
    await tapAndSettle(tester, key('outline-contents-chapter-c2'));
    expect(_strip('Chapter 2'), findsOneWidget);
    expect(tester.takeException(), isNull);
  });

  appTest('a jump highlights its target, and the highlight fades', (
    tester,
  ) async {
    await pumpApp(tester, mobile: false, empty: true);
    final store = await openAgentStore(tester);
    await _seed(tester, store);
    await _pumpChat(tester, store);

    await tapAndSettle(tester, key('outline-where'));
    await tester.tap(key('outline-contents-chapter-c2'));
    // The sheet closes and the jump lands a few frames later.
    final flash = find.ancestor(
      of: key('outline-chapter-c2'),
      matching: find.byType(OutlineFlash),
    );
    for (var i = 0; i < 60 && flash.evaluate().isEmpty; i++) {
      await tester.pump(const Duration(milliseconds: 16));
    }
    expect(flash, findsOneWidget);
    expect(_tint(tester, flash).a, greaterThan(0));

    await tester.pump(OutlineFlash.duration);
    expect(_tint(tester, flash).a, 0);
    expect(tester.takeException(), isNull);
  });

  appTest('a reference to a chat with no outline shows its target', (
    tester,
  ) async {
    await pumpApp(tester, mobile: false, empty: true);
    final store = await openAgentStore(tester);
    await _seed(tester, store);
    await _pumpChat(tester, store, outlined: false);
    // The chat is open and loaded: the request only sets the pending span.
    await store.openReference(store.openId!, 'u100', 'a100');
    await settle(tester);

    expect(key('agent-msg-u100'), findsOneWidget);
    expect(key('agent-msg-u0'), findsNothing);
    // The chat keeps its indexed list after the reference is taken.
    expect(find.byType(AnchoredList), findsOneWidget);
    expect(tester.takeException(), isNull);
  });

  appTest('a dock reference is taken by the dock, not the full screen', (
    tester,
  ) async {
    await pumpApp(tester, mobile: false, empty: true);
    final store = await openAgentStore(tester);
    await _seed(tester, store);
    await _pumpDockAndScreen(tester, store, outlined: false);
    await store.openReference(
      store.openId!,
      'u100',
      'a100',
      presentation: AgentPresentation.dock,
    );
    await settle(tester);

    final dock = key('dock-chat');
    final screen = key('screen-chat');
    expect(
      find.descendant(of: dock, matching: key('agent-msg-u100')),
      findsOneWidget,
    );
    expect(
      find.descendant(of: dock, matching: find.byType(AnchoredList)),
      findsOneWidget,
    );
    // The full screen never took the span: it is still the plain list, at the
    // end, with the far message unbuilt.
    expect(
      find.descendant(of: screen, matching: key('agent-msg-u100')),
      findsNothing,
    );
    expect(
      find.descendant(of: screen, matching: find.byType(AnchoredList)),
      findsNothing,
    );
    expect(store.pendingAnchor, isNull);
    expect(tester.takeException(), isNull);
  });

  appTest('a full-screen reference is taken by the screen, not the dock', (
    tester,
  ) async {
    await pumpApp(tester, mobile: false, empty: true);
    final store = await openAgentStore(tester);
    await _seed(tester, store);
    await _pumpDockAndScreen(tester, store, outlined: false);
    await store.openReference(
      store.openId!,
      'u100',
      'a100',
      presentation: AgentPresentation.screen,
    );
    await settle(tester);

    expect(
      find.descendant(of: key('screen-chat'), matching: key('agent-msg-u100')),
      findsOneWidget,
    );
    expect(
      find.descendant(of: key('dock-chat'), matching: key('agent-msg-u100')),
      findsNothing,
    );
    expect(
      find.descendant(
        of: key('dock-chat'),
        matching: find.byType(AnchoredList),
      ),
      findsNothing,
    );
    expect(store.pendingAnchor, isNull);
    expect(tester.takeException(), isNull);
  });

  for (final dark in [false, true]) {
    final theme = dark ? 'dark' : 'light';
    appTest('the folded card looks right in $theme', (tester) async {
      await pumpApp(tester, mobile: false, empty: true);
      final store = await openAgentStore(tester);
      await _seed(tester, store);
      await _pumpChat(tester, store, dark: dark);
      await expectLater(
        find.byKey(const ValueKey('outline-chapter-c0')),
        matchesGoldenFile('goldens/outline/folded_card_$theme.png'),
      );
    });

    appTest('the contents look right in $theme', (tester) async {
      await pumpApp(tester, mobile: false, empty: true);
      final store = await openAgentStore(tester);
      await _seed(tester, store);
      await _pumpChat(tester, store, dark: dark);
      await tapAndSettle(tester, key('outline-where'));
      await expectLater(
        find.byKey(const ValueKey('outline-contents')),
        matchesGoldenFile('goldens/outline/contents_$theme.png'),
      );
    });
  }
}
