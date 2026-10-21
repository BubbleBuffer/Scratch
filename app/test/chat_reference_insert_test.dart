import 'package:flutter/widgets.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:notes_app/core/models.dart';
import 'package:notes_app/editor/model.dart';
import 'package:notes_app/addons/agent/api.dart';
import 'package:notes_app/registry/block_registry.dart';
import 'package:notes_app/ui/ui.dart';

import 'chat_reference_fixture.dart';

class _Host implements BlockHost {
  @override
  void setTarget(Node node, String? target) => node.target = target;

  @override
  void setText(Node node, String text) => node.text = text;

  @override
  dynamic noSuchMethod(Invocation invocation) => super.noSuchMethod(invocation);
}

/// A chat list that needs no agent: one chat with the fixture chapter.
Future<List<ChatRefChat>> _loadOne() async => [
  ChatRefChat(
    sessionId: 'session',
    title: chatRefChatTitle,
    chapters: [chatRefChapter()],
  ),
];

/// Wraps [child] the way a page does: theme and a navigator for the panels.
Widget _app(Widget Function(BuildContext) home) => AppTheme(
  colors: AppColors.light,
  formFactor: FormFactor.desktop,
  child: WidgetsApp(
    color: AppColors.light.bgPrimary,
    pageRouteBuilder: <T>(settings, builder) => PageRouteBuilder<T>(
      settings: settings,
      pageBuilder: (context, _, _) => builder(context),
    ),
    home: Builder(builder: home),
  ),
);

BlockContext _block(BuildContext context, Node node, _Host host) =>
    BlockContext(
      context: context,
      node: node,
      host: host,
      style: ts(context.type.body, context.colors.textPrimary),
      lineHeight: 24,
    );

void main() {
  test(
    'the Chat block is a menu entry for the supported kind, insert-only',
    () {
      expect(chatRefBlock.kind, 'chat_ref');
      expect(chatRefBlock.kind, Kinds.chatRef);
      expect(chatRefBlock.label, 'Chat');
      expect(chatRefBlock.slashLabel, 'Chat');
      expect(chatRefBlock.inMenu, isTrue);
      expect(chatRefBlock.insertOnly, isTrue);
    },
  );

  testWidgets(
    'a new Chat block opens the picker and fills its range and summary',
    (tester) async {
      final node = Node(id: 'ref-node', kind: Kinds.chatRef);
      final chapter = chatRefChapter();
      final host = _Host();
      await tester.pumpWidget(
        AppTheme(
          colors: AppColors.light,
          formFactor: FormFactor.desktop,
          child: WidgetsApp(
            color: AppColors.light.bgPrimary,
            pageRouteBuilder: <T>(settings, builder) => PageRouteBuilder<T>(
              settings: settings,
              pageBuilder: (context, _, _) => builder(context),
            ),
            home: Builder(
              builder: (context) => ChatReferenceInsert(
                block: BlockContext(
                  context: context,
                  node: node,
                  host: host,
                  style: ts(context.type.body, context.colors.textPrimary),
                  lineHeight: 24,
                ),
                load: () async => [
                  ChatRefChat(
                    sessionId: 'session',
                    title: chatRefChatTitle,
                    chapters: [chapter],
                  ),
                ],
              ),
            ),
          ),
        ),
      );
      await tester.pumpAndSettle();
      await tester.tap(find.byKey(const ValueKey('picker-chat-session')));
      await tester.pumpAndSettle();
      await tester.tap(find.byKey(ValueKey('picker-chapter-${chapter.id}')));
      await tester.pumpAndSettle();
      final section = chapter.sections.last;
      await tester.tap(find.byKey(ValueKey('picker-section-${section.id}')));
      await tester.pumpAndSettle();
      final reference = ChatReference.fromTarget(node.target)!;
      expect(reference.sessionId, 'session');
      expect(reference.firstMessageId, section.firstMessageId);
      expect(reference.lastMessageId, section.lastMessageId);
      expect(reference.sectionTitle, section.title);
      expect(node.text, section.summary);
      await tester.pumpWidget(const SizedBox.shrink());
    },
  );

  testWidgets('whole chapter fills the chapter range, not a section', (
    tester,
  ) async {
    final node = Node(id: 'ref-node', kind: Kinds.chatRef);
    final host = _Host();
    await tester.pumpWidget(
      _app(
        (context) => ChatReferenceInsert(
          block: _block(context, node, host),
          load: _loadOne,
        ),
      ),
    );
    await tester.pumpAndSettle();
    await tester.tap(find.byKey(const ValueKey('picker-chat-session')));
    await tester.pumpAndSettle();
    await tester.tap(find.byKey(const ValueKey('picker-chapter-ch-2')));
    await tester.pumpAndSettle();
    await tester.tap(find.byKey(const ValueKey('picker-whole-chapter')));
    await tester.pumpAndSettle();

    final reference = ChatReference.fromTarget(node.target)!;
    expect(reference.level, ChatRefLevel.chapter);
    expect(reference.firstMessageId, 'm-40');
    expect(reference.lastMessageId, 'm-88');
    expect(node.text, chatRefChapter().summary);
    await tester.pumpWidget(const SizedBox.shrink());
  });

  testWidgets(
    'cancelling leaves an empty chooser that opens the picker again',
    (tester) async {
      final node = Node(id: 'ref-node', kind: Kinds.chatRef);
      final host = _Host();
      await tester.pumpWidget(
        _app(
          (context) => ChatReferenceInsert(
            block: _block(context, node, host),
            load: _loadOne,
          ),
        ),
      );
      await tester.pumpAndSettle();
      expect(find.byKey(const ValueKey('picker-chat-session')), findsOneWidget);

      tester.state<NavigatorState>(find.byType(Navigator).first).pop();
      await tester.pumpAndSettle();
      expect(node.target, isNull, reason: 'a cancel fills nothing');
      expect(
        find.byKey(const ValueKey('chatref-pick-ref-node')),
        findsOneWidget,
      );

      await tester.tap(find.byKey(const ValueKey('chatref-pick-ref-node')));
      await tester.pumpAndSettle();
      expect(find.byKey(const ValueKey('picker-chat-session')), findsOneWidget);
      await tester.pumpWidget(const SizedBox.shrink());
    },
  );

  testWidgets(
    'the Chat block body: a valid reference shows its card, a damaged one a placeholder',
    (tester) async {
      final valid = ChatReference.fromChapter(
        sessionId: 'sess-1',
        chatTitle: chatRefChatTitle,
        chapter: chatRefChapter(),
      );
      final node = Node(
        id: 'ok-node',
        kind: Kinds.chatRef,
        target: valid.toTarget(),
      );
      await tester.pumpWidget(
        _app((context) => chatRefBlock.body!(_block(context, node, _Host()))),
      );
      await tester.pumpAndSettle();
      expect(find.byType(ChatReferenceCard), findsOneWidget);
      expect(find.text('Chat reference unavailable'), findsNothing);

      final damaged = Node(
        id: 'bad-node',
        kind: Kinds.chatRef,
        target: '{"level":"galaxy"}',
      );
      await tester.pumpWidget(
        _app(
          (context) => chatRefBlock.body!(_block(context, damaged, _Host())),
        ),
      );
      await tester.pumpAndSettle();
      expect(find.text('Chat reference unavailable'), findsOneWidget);
      expect(find.byType(ChatReferenceCard), findsNothing);
      expect(find.byType(ChatReferenceInsert), findsNothing);
      await tester.pumpWidget(const SizedBox.shrink());
    },
  );
}
