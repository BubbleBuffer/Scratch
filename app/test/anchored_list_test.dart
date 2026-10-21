import 'package:flutter/widgets.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:notes_app/ui/anchored_list.dart';

void main() {
  testWidgets('jumps to an unbuilt variable-height row and scrolls both ways', (
    tester,
  ) async {
    final controller = AnchoredListController();
    addTearDown(controller.dispose);
    var builds = 0;
    int? top;
    await tester.pumpWidget(
      Directionality(
        textDirection: TextDirection.ltr,
        child: AnchoredList(
          controller: controller,
          itemCount: 1728,
          onTopIndexChanged: (index) => top = index,
          itemBuilder: (_, index) {
            builds++;
            return SizedBox(
              key: ValueKey('row-$index'),
              height: 40.0 + (index % 7) * 25,
              child: Text('Message $index'),
            );
          },
        ),
      ),
    );
    await tester.pump();
    expect(top, 0);
    expect(builds, lessThan(30));
    controller.jumpToIndex(1690);
    await tester.pump();
    await tester.pump();
    expect(top, 1690);
    expect(find.text('Message 1690'), findsOneWidget);
    expect(tester.getTopLeft(find.text('Message 1690')).dy, closeTo(0, 1));
    expect(builds, lessThan(60));
    controller.jumpToIndex(1691);
    await tester.pump();
    await tester.pump();
    expect(top, 1691);
    controller.scroll.jumpTo(-300);
    await tester.pump();
    await tester.pump();
    expect(top, lessThan(1691));
    controller.scroll.jumpTo(300);
    await tester.pump();
    await tester.pump();
    expect(top, greaterThan(1691));
    controller.jumpToIndex(0);
    await tester.pump();
    await tester.pump();
    expect(top, 0);
    controller.jumpToEnd();
    await tester.pump();
    await tester.pump();
    expect(find.text('Message 1727'), findsOneWidget);
    expect(
      tester.getBottomLeft(find.byType(AnchoredList)).dy -
          tester.getBottomLeft(find.byKey(const ValueKey('row-1727'))).dy,
      closeTo(0, 1),
    );
    await tester.pumpWidget(const SizedBox.shrink());
  });
}
