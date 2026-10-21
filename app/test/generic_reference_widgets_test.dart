// Behaviour of the generic reference widgets in ui/: the folding card, its
// footer action, the chip, the decision line, marked rows and the picker
// sheet. They take plain text and callbacks; no chat types are involved.

import 'package:flutter/widgets.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:notes_app/ui/ui.dart';

// The widgets read MediaQuery and the theme. The picker's search field needs
// an Overlay, which a WidgetsApp supplies through its Navigator, so the
// content goes in the app's home route.
Widget themed(
  Widget child, {
  bool dark = false,
  FormFactor formFactor = FormFactor.desktop,
}) => AppTheme(
  colors: dark ? AppColors.darkTheme : AppColors.light,
  formFactor: formFactor,
  child: WidgetsApp(
    color: const Color(0xFF000000),
    pageRouteBuilder: <T>(settings, builder) => PageRouteBuilder<T>(
      settings: settings,
      pageBuilder: (context, _, _) => builder(context),
    ),
    home: Center(child: SizedBox(width: 400, child: child)),
  ),
);

const _green = Color(0xFF6E9B5A);
const _plum = Color(0xFF8C6BB1);

void main() {
  testWidgets('a folded card shows its peek; tapping the face opens the body', (
    tester,
  ) async {
    await tester.pumpWidget(
      themed(
        FoldableStackCard(
          edge: _green,
          label: 'Chat › Chapter',
          title: 'Release plan',
          faceKey: const ValueKey('face'),
          peek: const Text('Peek line'),
          body: const Text('Body text'),
          footer: (open) => Text(open ? 'Foot open' : 'Foot folded'),
        ),
      ),
    );
    await tester.pumpAndSettle();

    expect(find.text('Chat › Chapter'), findsOneWidget);
    expect(find.text('Release plan'), findsOneWidget);
    expect(find.text('Peek line'), findsOneWidget);
    expect(find.text('Body text'), findsNothing);
    expect(find.text('Foot folded'), findsOneWidget);

    await tester.tap(find.byKey(const ValueKey('face')));
    await tester.pumpAndSettle();

    expect(find.text('Body text'), findsOneWidget);
    expect(find.text('Peek line'), findsNothing);
    expect(find.text('Foot open'), findsOneWidget);
  });

  testWidgets('a controlled card reports taps and opens only when told to', (
    tester,
  ) async {
    final reported = <bool>[];
    await tester.pumpWidget(
      themed(
        FoldableStackCard(
          edge: _green,
          title: 'Controlled',
          faceKey: const ValueKey('face'),
          body: const Text('Body text'),
          open: false,
          onOpenChanged: reported.add,
        ),
      ),
    );
    await tester.pumpAndSettle();

    await tester.tap(find.byKey(const ValueKey('face')));
    await tester.pumpAndSettle();
    expect(reported, [true]);
    expect(find.text('Body text'), findsNothing);
  });

  testWidgets('a parent that owns open applies it; taps only report', (
    tester,
  ) async {
    final reported = <bool>[];
    var open = false;
    await tester.pumpWidget(
      themed(
        StatefulBuilder(
          builder: (context, setLocal) => Column(
            mainAxisSize: MainAxisSize.min,
            crossAxisAlignment: CrossAxisAlignment.stretch,
            children: [
              FooterAction(
                key: const ValueKey('parent-open'),
                label: 'Open',
                onTap: () => setLocal(() => open = true),
              ),
              FooterAction(
                key: const ValueKey('parent-close'),
                label: 'Close',
                onTap: () => setLocal(() => open = false),
              ),
              FoldableStackCard(
                edge: _green,
                title: 'Parent owned',
                faceKey: const ValueKey('face'),
                body: const Text('Parent body'),
                open: open,
                onOpenChanged: reported.add,
              ),
            ],
          ),
        ),
      ),
    );
    await tester.pumpAndSettle();

    // A tap reports the wish but the parent has not applied it.
    await tester.tap(find.byKey(const ValueKey('face')));
    await tester.pumpAndSettle();
    expect(reported, [true]);
    expect(find.text('Parent body'), findsNothing);

    // The parent applies open; the card follows.
    await tester.tap(find.byKey(const ValueKey('parent-open')));
    await tester.pumpAndSettle();
    expect(find.text('Parent body'), findsOneWidget);

    // A tap now reports the close; the parent still owns the state.
    await tester.tap(find.byKey(const ValueKey('face')));
    await tester.pumpAndSettle();
    expect(reported, [true, false]);
    expect(find.text('Parent body'), findsOneWidget);

    await tester.tap(find.byKey(const ValueKey('parent-close')));
    await tester.pumpAndSettle();
    expect(find.text('Parent body'), findsNothing);
  });

  testWidgets('a footer action fires its tap', (tester) async {
    var taps = 0;
    await tester.pumpWidget(
      themed(
        FooterAction(
          key: const ValueKey('act'),
          label: 'Show messages',
          main: true,
          onTap: () => taps++,
        ),
      ),
    );
    expect(find.text('Show messages'), findsOneWidget);

    await tester.tap(find.byKey(const ValueKey('act')));
    expect(taps, 1);
  });

  testWidgets('a reference chip shows its label and fires its tap', (
    tester,
  ) async {
    var taps = 0;
    await tester.pumpWidget(
      themed(
        ReferenceChip(
          label: 'Cursor › Native drop',
          colour: _green,
          onTap: () => taps++,
        ),
      ),
    );
    expect(find.text('Cursor › Native drop'), findsOneWidget);

    await tester.tap(find.text('Cursor › Native drop'));
    expect(taps, 1);
  });

  testWidgets('a chip built from segments joins them with a chevron', (
    tester,
  ) async {
    await tester.pumpWidget(
      themed(
        const ReferenceChip(
          segments: ['Cursor', 'Native drop', 'Ghost text'],
          colour: _green,
        ),
      ),
    );
    expect(find.text('Cursor › Native drop › Ghost text'), findsOneWidget);
  });

  testWidgets('a peek decision is one clipped line with a ◆ mark', (
    tester,
  ) async {
    await tester.pumpWidget(
      themed(const DecisionLine('Keep the workaround', peek: true)),
    );
    expect(find.text('◆ '), findsOneWidget);
    expect(find.text('Keep the workaround'), findsOneWidget);
    expect(tester.widget<Text>(find.text('Keep the workaround')).maxLines, 1);
  });

  testWidgets('a boxed decision keeps its full text', (tester) async {
    const text = 'Ship for Chrome first; Explorer stays behind a flag.';
    await tester.pumpWidget(themed(const DecisionLine(text, boxed: true)));
    expect(find.text(text), findsOneWidget);
    expect(tester.widget<Text>(find.text(text)).maxLines, isNull);
  });

  testWidgets('marked rows: tappable rows fire, plain rows do not need a tap', (
    tester,
  ) async {
    final taps = <String>[];
    await tester.pumpWidget(
      themed(
        MarkedList(
          divided: true,
          rows: [
            MarkedRow(
              key: const ValueKey('row-a'),
              label: 'Alpha',
              colour: _green,
              onTap: () => taps.add('a'),
            ),
            MarkedRow(
              key: const ValueKey('row-b'),
              label: 'Beta',
              colour: _plum,
              dot: true,
              small: true,
              oneLine: true,
              trailing: const Text('here'),
              detail: const Text('Beta detail'),
            ),
          ],
        ),
      ),
    );

    expect(find.text('Beta'), findsOneWidget);
    expect(find.text('Beta detail'), findsOneWidget);
    expect(find.text('here'), findsOneWidget);

    await tester.tap(find.byKey(const ValueKey('row-a')));
    expect(taps, ['a']);
  });

  testWidgets('a picker shows its header, groups, list and footer', (
    tester,
  ) async {
    final query = TextEditingController();
    final focus = FocusNode();
    final typed = <String>[];
    try {
      await tester.pumpWidget(
        themed(
          PickerSheet(
            header: const Text('Pick one'),
            above: PickerGroup(
              label: 'Show as',
              children: [const Text('Card')],
            ),
            search: PickerSearch(
              controller: query,
              focusNode: focus,
              onChanged: typed.add,
              placeholder: 'Search',
              fieldKey: const ValueKey('picker-search'),
            ),
            list: Column(
              crossAxisAlignment: CrossAxisAlignment.stretch,
              children: [
                SheetItem(
                  key: const ValueKey('item-1'),
                  label: 'First',
                  onTap: () {},
                ),
              ],
            ),
            footer: const Text('Foot'),
          ),
        ),
      );

      expect(find.text('Pick one'), findsOneWidget);
      expect(find.text('Show as'), findsOneWidget);
      expect(find.text('Card'), findsOneWidget);
      expect(find.text('First'), findsOneWidget);
      expect(find.text('Foot'), findsOneWidget);

      await tester.enterText(find.byKey(const ValueKey('picker-search')), 'fi');
      expect(typed, ['fi']);
    } finally {
      // Unmount first, so no field still reads the controller or focus node.
      await tester.pumpWidget(const SizedBox());
      query.dispose();
      focus.dispose();
    }
  });

  testWidgets('a picker without search or footer has neither', (tester) async {
    await tester.pumpWidget(
      themed(
        PickerSheet(
          list: const Text('Only list'),
          listHeight: null,
          listKey: const ValueKey('list'),
        ),
      ),
    );
    expect(find.text('Only list'), findsOneWidget);
    expect(find.byKey(const ValueKey('list')), findsOneWidget);
    expect(find.byType(SearchField), findsNothing);
  });

  testWidgets('on a phone a picker opens as a sheet and a row pops it', (
    tester,
  ) async {
    Object? picked;
    await tester.pumpWidget(
      themed(
        Builder(
          builder: (context) => Hover(
            key: const ValueKey('open-picker'),
            onTap: () => showPickerSheet<String>(
              context,
              (ctx) => PickerSheet(
                header: const Text('Phone picker'),
                list: SheetItem(
                  key: const ValueKey('phone-pick'),
                  label: 'Pick me',
                  onTap: () => Navigator.of(ctx).pop('picked'),
                ),
              ),
            ).then((v) => picked = v),
            child: const Text('Open'),
          ),
        ),
        formFactor: FormFactor.mobile,
      ),
    );

    await tester.tap(find.byKey(const ValueKey('open-picker')));
    await tester.pumpAndSettle();
    expect(find.text('Phone picker'), findsOneWidget);

    await tester.tap(find.byKey(const ValueKey('phone-pick')));
    await tester.pumpAndSettle();
    expect(picked, 'picked');
    expect(find.text('Phone picker'), findsNothing);
  });
}
