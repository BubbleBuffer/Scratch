// The shared board deliberately runs without a vault, database, or bridge.
import 'package:flutter/gestures.dart';
import 'package:flutter/services.dart';
import 'package:flutter/widgets.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:notes_app/ui/board/board_renderer.dart';
import 'package:notes_app/ui/board/board_source.dart';
import 'package:notes_app/ui/ui.dart';

class MemoryBoardSource extends BoardSource {
  final titles = <String, String>{'todo': 'Todo', 'done': 'Done'};
  final order = <String>['todo', 'done'];
  final cards = <String, List<BoardCard>>{
    'todo': [
      const BoardCard(id: 'a', title: 'Alpha', peek: ['A note preview']),
      const BoardCard(id: 'b', title: 'Beta'),
      const BoardCard(id: 'c', title: 'Gamma'),
    ],
    'done': [const BoardCard(id: 'd', title: 'Delta')],
  };
  final requested = <int>[];
  String? opened;
  int openCount = 0;
  int _next = 0;

  @override
  List<BoardColumn> get columns => [
    for (final id in order)
      BoardColumn(
        id: id,
        title: titles[id]!,
        count: cards[id]!.length,
        color: const Color(0xFF697A6B),
        background: const Color(0xFFF3EFE7),
      ),
  ];
  @override
  int cardCount(String columnId) => cards[columnId]!.length;
  @override
  BoardCard? cardAt(String columnId, int index) {
    requested.add(index);
    return cards[columnId]![index];
  }

  @override
  Iterable<BoardCard> loadedCards(String columnId) => cards[columnId]!;
  @override
  Future<void> moveCard(
    String cardId,
    String fromColumnId,
    String toColumnId,
    String? afterCardId,
  ) async {
    final from = cards[fromColumnId]!;
    final card = from.removeAt(from.indexWhere((card) => card.id == cardId));
    final to = cards[toColumnId]!;
    final index = afterCardId == null
        ? 0
        : to.indexWhere((card) => card.id == afterCardId) + 1;
    to.insert(index, card);
    notifyListeners();
  }

  @override
  Future<void> addCard(BuildContext context, String columnId) async {
    cards[columnId]!.add(BoardCard(id: 'new-${_next++}', title: 'New note'));
    notifyListeners();
  }

  @override
  void openCard(BuildContext context, BoardCard card) {
    opened = card.id;
    openCount++;
  }

  @override
  Future<void> Function(String) get addColumn => (title) async {
    final id = 'column-${_next++}';
    titles[id] = title;
    order.add(id);
    cards[id] = [];
    notifyListeners();
  };
  @override
  Future<void> Function(String, String) get renameColumn => (id, title) async {
    titles[id] = title;
    notifyListeners();
  };
  @override
  Future<void> Function(String, String?) get reorderColumn =>
      (id, after) async {
        order.remove(id);
        order.insert(after == null ? 0 : order.indexOf(after) + 1, id);
        notifyListeners();
      };
}

class DeletableBoardSource extends MemoryBoardSource {
  final deleted = <String>[];

  @override
  Future<void> Function(String)? get deleteColumn => (id) async {
    deleted.add(id);
    order.remove(id);
    notifyListeners();
  };
}

Finder key(String value) => find.byKey(ValueKey(value));

Future<void> mount(
  WidgetTester tester,
  MemoryBoardSource source, {
  bool mobile = false,
  bool inline = false,
  BoardCardBuilder? cardBuilder,
}) async {
  tester.view.physicalSize = const Size(1100, 600);
  tester.view.devicePixelRatio = 1;
  addTearDown(tester.view.resetPhysicalSize);
  addTearDown(tester.view.resetDevicePixelRatio);
  addTearDown(source.dispose);
  await tester.pumpWidget(
    AppTheme(
      colors: AppColors.light,
      formFactor: mobile ? FormFactor.mobile : FormFactor.desktop,
      child: WidgetsApp(
        color: const Color(0xFFFFFFFF),
        onGenerateRoute: (_) => PageRouteBuilder<void>(
          pageBuilder: (context, animation, secondaryAnimation) =>
              BoardRenderer(
                source: source,
                columnWidth: 220,
                inline: inline,
                cardBuilder: cardBuilder,
              ),
        ),
      ),
    ),
  );
  await tester.pumpAndSettle();
}

Future<void> dragCard(
  WidgetTester tester,
  String title,
  Offset destination, {
  bool mobile = false,
}) async {
  final start = tester.getCenter(key('card-$title'));
  final gesture = await tester.startGesture(
    start,
    kind: mobile ? PointerDeviceKind.touch : PointerDeviceKind.mouse,
  );
  if (mobile) await tester.pump(const Duration(milliseconds: 600));
  for (var i = 1; i <= 12; i++) {
    await gesture.moveTo(Offset.lerp(start, destination, i / 12)!);
    await tester.pump(const Duration(milliseconds: 16));
  }
  await gesture.up();
  await tester.pumpAndSettle();
}

void main() {
  testWidgets(
    'renders column titles, colours, card titles and peeks without a database',
    (tester) async {
      final source = MemoryBoardSource();
      await mount(tester, source);
      expect(find.text('Todo'), findsOneWidget);
      expect(find.text('Done'), findsOneWidget);
      expect(find.text('Alpha'), findsOneWidget);
      expect(find.text('A note preview'), findsOneWidget);
      final column = tester.widget<Container>(key('board-col-Todo'));
      expect(
        (column.decoration! as BoxDecoration).color,
        const Color(0xFFF3EFE7),
      );
      expect(tester.takeException(), isNull);
    },
  );

  for (final mobile in [false, true]) {
    testWidgets(
      '${mobile ? 'mobile long press' : 'desktop drag'} moves a card across columns',
      (tester) async {
        final source = MemoryBoardSource();
        await mount(tester, source, mobile: mobile);
        if (mobile) {
          expect(
            find.byType(LongPressDraggable<BoardCardDrag>),
            findsNWidgets(4),
          );
        }
        await dragCard(
          tester,
          'Alpha',
          tester.getCenter(key('board-new-Done')),
          mobile: mobile,
        );
        expect(source.cards['todo']!.map((card) => card.id), ['b', 'c']);
        expect(source.cards['done']!.map((card) => card.id), ['d', 'a']);
        expect(
          find.descendant(
            of: key('board-col-Done'),
            matching: key('card-Alpha'),
          ),
          findsOneWidget,
        );
        expect(tester.takeException(), isNull);
      },
    );
  }

  testWidgets(
    'reorders cards before another card and at the end within a column',
    (tester) async {
      final source = MemoryBoardSource();
      await mount(tester, source);
      await dragCard(tester, 'Gamma', tester.getCenter(key('drop-a')));
      expect(source.cards['todo']!.map((card) => card.id), ['c', 'a', 'b']);
      await dragCard(tester, 'Gamma', tester.getCenter(key('board-new-Todo')));
      expect(source.cards['todo']!.map((card) => card.id), ['a', 'b', 'c']);
      expect(tester.takeException(), isNull);
    },
  );

  testWidgets('adds a card through the renderer and opens it', (tester) async {
    final source = MemoryBoardSource();
    await mount(tester, source);
    await tester.tap(key('board-new-Done'));
    await tester.pumpAndSettle();
    expect(
      find.descendant(
        of: key('board-col-Done'),
        matching: key('card-New note'),
      ),
      findsOneWidget,
    );
    await tester.tap(key('card-New note'));
    expect(source.opened, source.cards['done']!.last.id);
    expect(tester.takeException(), isNull);
  });

  testWidgets('adds, renames and reorders columns through renderer controls', (
    tester,
  ) async {
    final source = MemoryBoardSource();
    await mount(tester, source, mobile: true);
    await tester.tap(key('board-add-column'));
    await tester.pumpAndSettle();
    expect(find.text('New column'), findsNWidgets(2));

    await tester.tap(key('group-menu-Todo'));
    await tester.pumpAndSettle();
    await tester.enterText(key('board-column-title-todo'), 'Ideas');
    await tester.testTextInput.receiveAction(TextInputAction.done);
    await tester.pumpAndSettle();
    expect(source.titles['todo'], 'Ideas');
    expect(key('board-col-Ideas'), findsOneWidget);

    await tester.tap(key('board-column-right-todo'));
    await tester.pumpAndSettle();
    expect(source.order.take(2), ['done', 'todo']);
    expect(
      tester.getTopLeft(key('board-col-Done')).dx,
      lessThan(tester.getTopLeft(key('board-col-Ideas')).dx),
    );
    expect(tester.takeException(), isNull);
  });

  testWidgets(
    'card keyboard activation is shared by a source without a database',
    (tester) async {
      final source = MemoryBoardSource();
      await mount(tester, source);
      final boundary = tester.widget<FocusableActionDetector>(
        find
            .descendant(
              of: key('board-card-a'),
              matching: find.byType(FocusableActionDetector),
            )
            .first,
      );
      boundary.focusNode!.requestFocus();
      await tester.pump();
      await tester.sendKeyEvent(LogicalKeyboardKey.enter);
      expect(source.opened, 'a');
      source.opened = null;
      await tester.sendKeyEvent(LogicalKeyboardKey.space);
      expect(source.opened, 'a');
    },
  );

  testWidgets(
    'custom noninteractive cards use renderer keyboard and pointer activation',
    (tester) async {
      final source = MemoryBoardSource();
      await mount(
        tester,
        source,
        cardBuilder: (context, column, card, lifted) =>
            SizedBox(height: 40, child: Text(card.title)),
      );
      final boundary = tester.widget<FocusableActionDetector>(
        find
            .descendant(
              of: key('board-card-a'),
              matching: find.byType(FocusableActionDetector),
            )
            .first,
      );
      boundary.focusNode!.requestFocus();
      await tester.pump();
      await tester.sendKeyEvent(LogicalKeyboardKey.enter);
      expect(source.opened, 'a');
      expect(source.openCount, 1);
      await tester.sendKeyEvent(LogicalKeyboardKey.space);
      expect(source.openCount, 2);
      await tester.tap(find.text('Alpha'));
      expect(source.openCount, 3);
      expect(tester.takeException(), isNull);
    },
  );

  testWidgets(
    'interactive custom cards retain pointer handling and have one keyboard open',
    (tester) async {
      final source = MemoryBoardSource();
      var customOpens = 0;
      await mount(
        tester,
        source,
        cardBuilder: (context, column, card, lifted) => Hover(
          onTap: () => customOpens++,
          child: SizedBox(height: 40, child: Text(card.title)),
        ),
      );
      final boundary = tester.widget<FocusableActionDetector>(
        find
            .descendant(
              of: key('board-card-a'),
              matching: find.byType(FocusableActionDetector),
            )
            .first,
      );
      boundary.focusNode!.requestFocus();
      await tester.pump();
      expect(
        Focus.of(tester.element(find.text('Alpha'))).canRequestFocus,
        isFalse,
      );
      await tester.sendKeyEvent(LogicalKeyboardKey.enter);
      await tester.sendKeyEvent(LogicalKeyboardKey.space);
      expect(source.openCount, 2);
      expect(customOpens, 0);
      await tester.tap(find.text('Alpha'));
      expect(customOpens, 1);
      expect(source.openCount, 2);
      expect(tester.takeException(), isNull);
    },
  );

  testWidgets(
    'large columns build lazily and inline columns expose load more',
    (tester) async {
      final source = MemoryBoardSource();
      source.cards['todo'] = [
        for (var i = 0; i < 1000; i++) BoardCard(id: 'n$i', title: 'Note $i'),
      ];
      await mount(tester, source);
      expect(source.requested.reduce((a, b) => a > b ? a : b), lessThan(50));
      expect(key('card-Note 999'), findsNothing);
      await tester.pumpWidget(const SizedBox());
      source.requested.clear();
      await tester.pumpWidget(
        WidgetsApp(
          color: const Color(0xFFFFFFFF),
          onGenerateRoute: (_) => PageRouteBuilder<void>(
            pageBuilder: (context, a, b) => AppTheme(
              colors: AppColors.light,
              formFactor: FormFactor.desktop,
              child: SingleChildScrollView(
                child: BoardRenderer(
                  source: source,
                  inline: true,
                  columnWidth: 220,
                ),
              ),
            ),
          ),
        ),
      );
      await tester.pumpAndSettle();
      expect(key('card-Note 24'), findsOneWidget);
      expect(key('card-Note 25'), findsNothing);
      await tester.ensureVisible(key('load-more'));
      await tester.tap(key('load-more'));
      await tester.pumpAndSettle();
      expect(key('card-Note 49'), findsOneWidget);
      expect(key('card-Note 50'), findsNothing);
      expect(tester.takeException(), isNull);
    },
  );

  testWidgets(
    'header menu offers delete column when the hook has two columns',
    (tester) async {
      final source = DeletableBoardSource();
      await mount(tester, source, mobile: true);
      await tester.tap(key('group-menu-Todo'));
      await tester.pumpAndSettle();
      expect(key('group-rename'), findsOneWidget);
      expect(key('group-delete'), findsOneWidget);
      expect(find.text('Delete column'), findsOneWidget);
      expect(tester.takeException(), isNull);
    },
  );

  testWidgets('delete column is hidden when only one column remains', (
    tester,
  ) async {
    final source = DeletableBoardSource()..order.remove('done');
    await mount(tester, source, mobile: true);
    await tester.tap(key('group-menu-Todo'));
    await tester.pumpAndSettle();
    expect(find.text('Delete column'), findsNothing);
    // Without a delete choice the button keeps its direct rename behaviour.
    expect(key('board-column-title-todo'), findsOneWidget);
    expect(tester.takeException(), isNull);
  });

  testWidgets('null delete hook keeps direct rename with no delete item', (
    tester,
  ) async {
    final source = MemoryBoardSource();
    await mount(tester, source, mobile: true);
    await tester.tap(key('group-menu-Todo'));
    await tester.pumpAndSettle();
    expect(find.text('Delete column'), findsNothing);
    expect(key('group-rename'), findsNothing);
    expect(key('board-column-title-todo'), findsOneWidget);
    expect(tester.takeException(), isNull);
  });

  testWidgets('confirming delete calls the hook with the column id', (
    tester,
  ) async {
    final source = DeletableBoardSource();
    await mount(tester, source, mobile: true);
    await tester.tap(key('group-menu-Todo'));
    await tester.pumpAndSettle();
    await tester.tap(key('group-delete'));
    await tester.pumpAndSettle();
    expect(find.text('Delete "Todo"?'), findsOneWidget);
    expect(
      find.textContaining('move to "Done", the first remaining column.'),
      findsOneWidget,
    );
    await tester.tap(key('confirm-ok'));
    await tester.pumpAndSettle();
    expect(source.deleted, ['todo']);
    expect(source.order, ['done']);
    expect(key('board-col-Todo'), findsNothing);
    expect(tester.takeException(), isNull);
  });

  testWidgets('cancelling delete leaves the column and cards alone', (
    tester,
  ) async {
    final source = DeletableBoardSource();
    await mount(tester, source, mobile: true);
    await tester.tap(key('group-menu-Todo'));
    await tester.pumpAndSettle();
    await tester.tap(key('group-delete'));
    await tester.pumpAndSettle();
    await tester.tap(key('confirm-cancel'));
    await tester.pumpAndSettle();
    expect(source.deleted, isEmpty);
    expect(source.order, ['todo', 'done']);
    expect(source.cards['todo']!.length, 3);
    expect(key('board-col-Todo'), findsOneWidget);
    expect(tester.takeException(), isNull);
  });
}
