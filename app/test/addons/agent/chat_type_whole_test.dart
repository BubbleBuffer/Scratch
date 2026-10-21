// The chat note type's builders for a whole chat, drawn from a bare session ID.
// Range payloads keep their chapter or section card. Reads use a fake source
// so no agent runs.

import 'package:flutter/widgets.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:notes_app/addons/agent/api.dart';
import 'package:notes_app/addons/agent/chat_type.dart';
import 'package:notes_app/core/models.dart';
import 'package:notes_app/registry/addon_registry.dart';
import 'package:notes_app/registry/note_type_registry.dart';
import 'package:notes_app/src/rust/api/agent.dart';
import 'package:notes_app/ui/embed.dart';
import 'package:notes_app/ui/ui.dart';

import '../../chat_reference_fixture.dart';

class _Source implements ChatReferenceSource {
  _Source(this.snapshot);

  ChatReferenceSnapshot? snapshot;
  @override
  bool enabled = true;
  final requested = <String>[];

  @override
  Future<ChatReferenceSnapshot?> load(String sessionId) async {
    requested.add(sessionId);
    return snapshot;
  }
}

class _Unreachable implements ChatReferenceSource {
  @override
  bool get enabled => true;

  @override
  Future<ChatReferenceSnapshot?> load(String sessionId) async =>
      throw StateError('Agent unreachable');
}

OutlineChapter _chapter(String id, String title) => OutlineChapter(
  id: id,
  title: title,
  summary: 'Summary of $title.',
  decisions: const [],
  ongoing: false,
  startedAt: '2026-09-05T10:00:00Z',
  endedAt: '2026-09-06T10:00:00Z',
  firstMessageId: 'm-1',
  lastMessageId: 'm-2',
  sections: const [],
);

ChatReferenceSnapshot _snapshot({List<OutlineChapter>? chapters}) =>
    ChatReferenceSnapshot(
      chatTitle: chatRefChatTitle,
      messageIds: [for (var i = 40; i <= 88; i++) 'm-$i'],
      outline: ChatOutline(
        chapters: chapters ?? [chatRefChapter(), _chapter('ch-9', 'Second')],
      ),
    );

Widget _app(Widget child) => AppTheme(
  colors: AppColors.light,
  formFactor: FormFactor.desktop,
  child: WidgetsApp(
    color: AppColors.light.bgPrimary,
    pageRouteBuilder: <T>(settings, builder) => PageRouteBuilder<T>(
      settings: settings,
      pageBuilder: (context, _, _) => builder(context),
    ),
    home: Center(child: SizedBox(width: 420, child: child)),
  ),
);

/// Builds the embed or card of the chat type inside the app's theme.
Widget _builder(NoteArgs args, {bool card = false}) => _app(
  Builder(
    builder: (context) =>
        card ? chatCard(context, args) : chatEmbed(context, args),
  ),
);

Future<void> _settle(WidgetTester tester) async {
  await tester.pump();
  await tester.pump(const Duration(milliseconds: 300));
}

/// Opens the card and waits out its size animation. The body's AnimatedSize
/// starts ticking on the frame after the tap, so one frame goes first; until
/// it has grown, the footer lies outside the box and takes no taps.
Future<void> _open(WidgetTester tester) async {
  await tester.tap(find.byKey(const ValueKey('chatwhole-fold')));
  await tester.pump();
  await tester.pump(const Duration(milliseconds: 400));
}

void main() {
  testWidgets(
    'a bare session ID draws the whole chat: its title and chapters',
    (tester) async {
      final source = _Source(_snapshot());
      await tester.pumpWidget(
        _app(ChatSessionCard(sessionId: 'sess-1', source: source)),
      );
      await _settle(tester);

      expect(source.requested, ['sess-1']);
      expect(find.text(chatRefChatTitle), findsOneWidget);
      expect(find.text('2 chapters'), findsOneWidget);

      await _open(tester);
      final first = outlineNameGist(
        chatRefChapter().title,
        chatRefChapter().summary,
      ).name;
      final second = outlineNameGist('Second', 'Summary of Second.').name;
      expect(find.text(first), findsOneWidget);
      expect(find.text(second), findsOneWidget);
      expect(find.text('chat unavailable'), findsNothing);
      await tester.pumpWidget(const SizedBox.shrink());
    },
  );

  testWidgets(
    'a whole chat with no chapters shows its title and a count of zero',
    (tester) async {
      final source = _Source(_snapshot(chapters: const []));
      await tester.pumpWidget(
        _app(ChatSessionCard(sessionId: 'sess-1', source: source)),
      );
      await _settle(tester);

      expect(find.text(chatRefChatTitle), findsOneWidget);
      expect(find.text('0 chapters'), findsOneWidget);
      await tester.pumpWidget(const SizedBox.shrink());
    },
  );

  testWidgets('a missing chat shows the unavailable placeholder, not a card', (
    tester,
  ) async {
    final source = _Source(null);
    await tester.pumpWidget(
      _app(ChatSessionCard(sessionId: 'gone', source: source)),
    );
    await _settle(tester);

    expect(source.requested, ['gone']);
    expect(find.text('Chat unavailable'), findsOneWidget);
    expect(find.byType(FoldableStackCard), findsNothing);
    await tester.pumpWidget(const SizedBox.shrink());
  });

  testWidgets(
    'an unreachable agent with no earlier read shows the placeholder',
    (tester) async {
      await tester.pumpWidget(
        _app(ChatSessionCard(sessionId: 'sess-1', source: _Unreachable())),
      );
      await _settle(tester);

      expect(find.text('Chat unavailable'), findsOneWidget);
      expect(find.byType(FoldableStackCard), findsNothing);
      await tester.pumpWidget(const SizedBox.shrink());
    },
  );

  testWidgets('with the agent off and no read, the whole chat is not drawn', (
    tester,
  ) async {
    final source = _Source(_snapshot())..enabled = false;
    await tester.pumpWidget(
      _app(ChatSessionCard(sessionId: 'sess-1', source: source)),
    );
    await _settle(tester);

    expect(source.requested, isEmpty);
    expect(find.text('Chat unavailable'), findsOneWidget);
    expect(find.byType(FoldableStackCard), findsNothing);
    await tester.pumpWidget(const SizedBox.shrink());
  });

  testWidgets('Open in chat runs the given action', (tester) async {
    var opened = 0;
    await tester.pumpWidget(
      _app(
        ChatSessionCard(
          sessionId: 'sess-1',
          source: _Source(_snapshot()),
          onOpen: () => opened++,
        ),
      ),
    );
    await _settle(tester);
    await _open(tester);

    await tester.tap(find.byKey(const ValueKey('chatwhole-open')));
    await tester.pump();
    expect(opened, 1);
    await tester.pumpWidget(const SizedBox.shrink());
  });

  testWidgets(
    'an embed with a bare ID target names that session as a whole chat',
    (tester) async {
      await tester.pumpWidget(
        _builder(const NoteArgs(pageId: 'sess-1', target: 'sess-1')),
      );
      await _settle(tester);

      final card = tester.widget<ChatSessionCard>(find.byType(ChatSessionCard));
      expect(card.sessionId, 'sess-1');
      expect(find.byType(ChatReferenceCard), findsNothing);
      await tester.pumpWidget(const SizedBox.shrink());
    },
  );

  testWidgets('an embed with no target uses the page ID as the whole chat', (
    tester,
  ) async {
    await tester.pumpWidget(_builder(const NoteArgs(pageId: 'sess-2')));
    await _settle(tester);

    final card = tester.widget<ChatSessionCard>(find.byType(ChatSessionCard));
    expect(card.sessionId, 'sess-2');
    await tester.pumpWidget(const SizedBox.shrink());
  });

  testWidgets('a card with a bare ID target draws the whole chat', (
    tester,
  ) async {
    await tester.pumpWidget(
      _builder(const NoteArgs(pageId: 'sess-1', target: 'sess-1'), card: true),
    );
    await _settle(tester);

    final card = tester.widget<ChatSessionCard>(find.byType(ChatSessionCard));
    expect(card.sessionId, 'sess-1');
    await tester.pumpWidget(const SizedBox.shrink());
  });

  testWidgets(
    'a range payload keeps the chapter card through the embed and the card',
    (tester) async {
      final ref = ChatReference.fromChapter(
        sessionId: 'sess-1',
        chatTitle: chatRefChatTitle,
        chapter: chatRefChapter(),
      );
      for (final card in [false, true]) {
        await tester.pumpWidget(
          _builder(
            NoteArgs(
              pageId: 'sess-1',
              pageType: 'chat',
              target: ref.toTarget(),
            ),
            card: card,
          ),
        );
        await _settle(tester);

        expect(find.byType(ChatReferenceCard), findsOneWidget);
        expect(find.byType(ChatSessionCard), findsNothing);
        expect(find.textContaining(chatRefChatTitle), findsOneWidget);
      }
      await tester.pumpWidget(const SizedBox.shrink());
    },
  );

  testWidgets('a damaged range payload keeps the reference placeholder', (
    tester,
  ) async {
    await tester.pumpWidget(
      _builder(const NoteArgs(pageId: 'sess-1', target: '{"v":1,"level":')),
    );
    await _settle(tester);

    expect(find.text('Chat reference unavailable'), findsOneWidget);
    expect(find.byType(ChatSessionCard), findsNothing);
    await tester.pumpWidget(const SizedBox.shrink());
  });

  testWidgets('a card with no target and no session ID keeps the chat chip', (
    tester,
  ) async {
    await tester.pumpWidget(_builder(const NoteArgs(pageId: ''), card: true));
    await _settle(tester);

    expect(find.byType(ChatSessionCard), findsNothing);
    expect(find.byType(ChatReferenceCard), findsNothing);
    expect(find.text('Untitled chat'), findsOneWidget);
    await tester.pumpWidget(const SizedBox.shrink());
  });

  testWidgets(
    'the generic embed of a chat index row draws the whole chat through the registered agent',
    (tester) async {
      final ctx = AddonContext();
      addTearDown(ctx.noteTypes.dispose);
      bootAddons(const [AgentAddon()], ctx);
      await tester.pumpWidget(
        _app(
          EmbedNote(
            targetId: 'chat:sess-1',
            rootPageId: 'A',
            addons: ctx,
            loader: (id) async => EmbedNoteData.fromHit(
              Hit(
                id: id,
                title: 'Stored chat title',
                snippet: 'Indexed summary',
                fuzzy: false,
                source: 'agent',
                pageType: 'chat',
              ),
            ),
          ),
        ),
      );
      await _settle(tester);

      final card = tester.widget<ChatSessionCard>(find.byType(ChatSessionCard));
      expect(card.sessionId, 'sess-1');
      expect(find.byType(ChatReferenceCard), findsNothing);
      expect(
        find.text('Stored chat title'),
        findsNothing,
        reason: 'the whole chat takes its title from the agent, not the kit',
      );
      await tester.pumpWidget(const SizedBox.shrink());
    },
  );

  testWidgets('a chat index row in the page ID names the bare session', (
    tester,
  ) async {
    await tester.pumpWidget(_builder(const NoteArgs(pageId: 'chat:sess-1')));
    await _settle(tester);

    final card = tester.widget<ChatSessionCard>(find.byType(ChatSessionCard));
    expect(card.sessionId, 'sess-1');
    await tester.pumpWidget(const SizedBox.shrink());
  });

  testWidgets('a chat index row in the target names the bare session', (
    tester,
  ) async {
    await tester.pumpWidget(
      _builder(
        const NoteArgs(pageId: 'chat:sess-1', target: 'chat:sess-1'),
        card: true,
      ),
    );
    await _settle(tester);

    final card = tester.widget<ChatSessionCard>(find.byType(ChatSessionCard));
    expect(card.sessionId, 'sess-1');
    await tester.pumpWidget(const SizedBox.shrink());
  });

  testWidgets('a range payload keeps its card when the page ID is a chat row', (
    tester,
  ) async {
    final ref = ChatReference.fromChapter(
      sessionId: 'sess-1',
      chatTitle: chatRefChatTitle,
      chapter: chatRefChapter(),
    );
    await tester.pumpWidget(
      _builder(
        NoteArgs(
          pageId: 'chat:sess-1',
          pageType: 'chat',
          target: ref.toTarget(),
        ),
      ),
    );
    await _settle(tester);

    expect(find.byType(ChatReferenceCard), findsOneWidget);
    expect(find.byType(ChatSessionCard), findsNothing);
    await tester.pumpWidget(const SizedBox.shrink());
  });

  testWidgets('an empty chat row names no session and keeps the placeholder', (
    tester,
  ) async {
    await tester.pumpWidget(_builder(const NoteArgs(pageId: 'chat:')));
    await _settle(tester);

    expect(find.byType(ChatSessionCard), findsNothing);
    expect(find.text('Chat reference unavailable'), findsOneWidget);
    await tester.pumpWidget(const SizedBox.shrink());
  });
}
