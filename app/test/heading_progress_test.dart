import 'package:flutter/widgets.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:notes_app/blocks/heading.dart';
import 'package:notes_app/blocks/toggle.dart';
import 'package:notes_app/core/models.dart';
import 'package:notes_app/editor/model.dart';
import 'package:notes_app/registry/block_registry.dart';
import 'package:notes_app/ui/ui.dart';

Node node(String id, String kind, {bool done = false}) =>
    Node(id: id, kind: kind, checked: done);

class ProgressHost implements BlockHost, HeadingProgressHost {
  ProgressHost(this.headingProgressOrder);
  @override
  final List<Node> headingProgressOrder;
  bool open = false;
  VoidCallback? onToggle;
  @override
  bool isOpen(Node n) => open;
  @override
  void toggleOpen(Node n) {
    open = !open;
    onToggle?.call();
  }

  @override
  String? get headingProgressSpaceColor => 'moss';
  @override
  dynamic noSuchMethod(Invocation invocation) => super.noSuchMethod(invocation);
}

void main() {
  test(
    'heading sections count subordinate tasks until a same or higher heading',
    () {
      final h = node('section', Kinds.heading2);
      final order = [
        h,
        node('one', Kinds.todo, done: true),
        node('lower', Kinds.heading3),
        node('two', Kinds.todo),
        node('three', Kinds.todo, done: true),
        node('end', Kinds.heading2),
        node('outside', Kinds.todo),
      ];
      final progress = headingProgressOf(h, order)!;
      expect([progress.done, progress.total], [2, 3]);
      expect(progress.fraction, closeTo(2 / 3, .001));
      order[5].kind = Kinds.toggleHeading1;
      expect(headingProgressOf(h, order)!.total, 3);
    },
  );
  test(
    'a section needs at least two tasks and respects a toggle heading boundary',
    () {
      final h = node('section', Kinds.heading1);
      final one = node('one', Kinds.todo);
      expect(headingProgressOf(h, [h, one]), isNull);
      expect(headingProgressOf(h, [one]), isNull);
      expect(
        headingProgressOf(h, [
          h,
          one,
          node('stop', Kinds.toggleHeading1),
          node('two', Kinds.todo),
        ]),
        isNull,
      );
      expect(
        headingProgressOf(h, [h, one, node('hidden', Kinds.todo)])!.total,
        2,
      );
    },
  );
  testWidgets(
    'a toggle heading keeps progress while its triangle opens the section',
    (tester) async {
      final h = node('fold', Kinds.toggleHeading2)..text = 'Packing';
      final one = node('one', Kinds.todo, done: true),
          two = node('two', Kinds.todo);
      h.children.addAll([one, two]);
      final host = ProgressHost([h, one, two]);
      await tester.pumpWidget(
        MediaQuery(
          data: const MediaQueryData(),
          child: AppTheme(
            colors: AppColors.light,
            formFactor: FormFactor.desktop,
            child: Directionality(
              textDirection: TextDirection.ltr,
              child: Center(
                child: SizedBox(
                  width: 460,
                  child: StatefulBuilder(
                    builder: (context, setState) {
                      host.onToggle = () => setState(() {});
                      final style = toggleHeading2Block.textStyle(
                        h,
                        context.colors,
                        context.type,
                      );
                      return toggleHeading2Block.decorate!(
                        BlockContext(
                          context: context,
                          node: h,
                          host: host,
                          style: style,
                          lineHeight: 31.2,
                        ),
                        Text(h.text, style: style),
                      );
                    },
                  ),
                ),
              ),
            ),
          ),
        ),
      );
      expect(find.text('1 of 2'), findsOneWidget);
      expect(host.open, isFalse);
      await tester.tap(find.byKey(const ValueKey('toggle-fold')));
      await tester.pumpAndSettle();
      expect(host.open, isTrue);
      expect(find.text('1 of 2'), findsOneWidget);
      expect(tester.takeException(), isNull);
    },
  );

  for (final reduced in [false, true]) {
    testWidgets('heading progress updates from a task tick, reduced=$reduced', (
      tester,
    ) async {
      final h = node('section', Kinds.heading2)..text = 'Packing';
      final task = node('one', Kinds.todo, done: true);
      final host = ProgressHost([
        h,
        task,
        node('two', Kinds.todo),
        node('three', Kinds.todo),
      ]);
      await tester.pumpWidget(
        MediaQuery(
          data: MediaQueryData(disableAnimations: reduced),
          child: AppTheme(
            colors: AppColors.light,
            formFactor: FormFactor.desktop,
            child: Directionality(
              textDirection: TextDirection.ltr,
              child: Center(
                child: SizedBox(
                  width: 340,
                  child: StatefulBuilder(
                    builder: (context, setState) {
                      final style = heading2Block.textStyle(
                        h,
                        context.colors,
                        context.type,
                      );
                      return Column(
                        mainAxisSize: MainAxisSize.min,
                        children: [
                          heading2Block.decorate!(
                            BlockContext(
                              context: context,
                              node: h,
                              host: host,
                              style: style,
                              lineHeight: 31.2,
                            ),
                            Text(h.text, style: style),
                          ),
                          StacksCheckbox(
                            key: const ValueKey('tick'),
                            checked: task.checked,
                            onChanged: (value) =>
                                setState(() => task.checked = value),
                          ),
                        ],
                      );
                    },
                  ),
                ),
              ),
            ),
          ),
        ),
      );
      expect(find.text('1 of 3'), findsOneWidget);
      final fill = find.descendant(
        of: find.byType(HeadingProgressBar),
        matching: find.byType(FractionallySizedBox),
      );
      expect(tester.getSize(fill).height, 6);
      expect(
        tester.getSize(fill).width,
        closeTo(tester.getSize(find.byType(HeadingProgressBar)).width / 3, .1),
      );
      expect(
        tester.getTopLeft(find.byType(HeadingProgressBar)).dx -
            tester.getTopRight(find.text('Packing')).dx,
        closeTo(12, .1),
      );
      await tester.tap(find.byKey(const ValueKey('tick')));
      await tester.pumpAndSettle();
      expect(find.text('0 of 3'), findsOneWidget);
      expect(h.text, 'Packing');
      expect(h.children, isEmpty);
      expect(tester.takeException(), isNull);
    });
  }
}
