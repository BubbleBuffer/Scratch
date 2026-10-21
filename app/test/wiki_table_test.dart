// The table block: GFM pipe tables drawn from their markdown text, inline
// marks in cells, narrow-width scrolling, the code block's language label, the
// markdown codecs of both kinds, and the unknown-kind placeholder.

import 'package:flutter/widgets.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:notes_app/blocks/blocks.dart';
import 'package:notes_app/blocks/code.dart';
import 'package:notes_app/blocks/table.dart';
import 'package:notes_app/core/models.dart';
import 'package:notes_app/editor/clipboard.dart';
import 'package:notes_app/editor/model.dart';
import 'package:notes_app/registry/block_registry.dart';
import 'package:notes_app/ui/ui.dart';

const _basic = '''
| Item | Count |
| --- | --- |
| Apple | 3 |
| Pear | 12 |''';

const _aligned = '''
| Left | Center | Right | Plain |
| :--- | :----: | ----: | ----- |
| l1 | c1 | r1 | p1 |''';

const _marks = r'''
| Style | Example |
| --- | --- |
| bold | **Bold** |
| italic | _Italic_ |
| code | `a \| b` |
| pipe in code | `x|y` |
| escaped | a \| b |
| link | [site](https://x.test) |
| mail | [write](mailto:me@x.test) |
| file | [notes](notes/a.md) |
| plain | snake_case and *unclosed |''';

const _pageLinks = r'''
| Page | Wiki |
| --- | --- |
| [plan](plan.md) | [[Road\|Highway]] |
| [gone](gone.md) | [[Missing]] |''';

const _wide = '''
| Column one | Column two | Column three | Column four |
| --- | --- | --- | --- |
| a long cell with plenty of words in it | another long cell | third | fourth |''';

/// A page link as the core marks it: [label] (its first place in [text]) gets a
/// Link to [target] and a PageLink to the page id [page].
List<BMark> _pageMarks(String text, String label, String target, String page) {
  final start = text.indexOf(label);
  if (start < 0) fail('"$label" is not in the table text');
  final end = start + label.length;
  return [
    BMark(start: start, end: end, kind: BMarkKind.link, url: target),
    BMark(start: start, end: end, kind: BMarkKind.pageLink, url: page),
  ];
}

class _Host implements BlockHost {
  /// Page ids passed to openPage, in order.
  final opened = <String>[];

  @override
  void openPage(String id) => opened.add(id);

  @override
  dynamic noSuchMethod(Invocation invocation) =>
      super.noSuchMethod(invocation);
}

/// The block as the editor draws it: its body, or its decorated text for a
/// block that is editable.
Widget _render(BuildContext context, Node node, BlockType type, {BlockHost? host}) {
  final b = BlockContext(
    context: context,
    node: node,
    host: host ?? _Host(),
    style: type.textStyle(node, context.colors, context.type),
    lineHeight: 24,
  );
  final decorate = type.decorate;
  if (decorate != null) return decorate(b, Text(node.text, style: b.style));
  return type.body!(b);
}

/// [child] at exactly [width], pinned to the top left. The page's own
/// constraints may not hand a plain SizedBox its width, so the Align makes the
/// block's width the real one.
Widget _sized(double width, Widget child) => Align(
  alignment: Alignment.topLeft,
  child: SizedBox(width: width, child: child),
);

Widget _app(Widget child, {FormFactor formFactor = FormFactor.desktop}) =>
    AppTheme(
      colors: AppColors.light,
      formFactor: formFactor,
      child: WidgetsApp(
        color: AppColors.light.bgPrimary,
        pageRouteBuilder: <T>(settings, builder) => PageRouteBuilder<T>(
          settings: settings,
          pageBuilder: (context, _, _) => builder(context),
        ),
        home: child,
      ),
    );

Future<void> _pumpTable(
  WidgetTester tester,
  String text, {
  double width = 700,
  FormFactor formFactor = FormFactor.desktop,
  List<BMark> marks = const [],
  BlockHost? host,
}) async {
  final node = Node(id: 'table-1', kind: Kinds.table, text: text, marks: List.of(marks));
  await tester.pumpWidget(
    _app(
      _sized(
        width,
        Builder(
          builder: (context) => _render(context, node, tableBlock, host: host),
        ),
      ),
      formFactor: formFactor,
    ),
  );
}

/// The span whose text is [text], searched through every Text on screen.
TextSpan _leaf(WidgetTester tester, String text) {
  for (final w in tester.widgetList<Text>(find.byType(Text))) {
    final hit = _find(w.textSpan, text);
    if (hit != null) return hit;
  }
  fail('no span reads "$text"');
}

TextSpan? _find(InlineSpan? span, String text) {
  if (span is! TextSpan) return null;
  if (span.text == text) return span;
  for (final child in span.children ?? const <InlineSpan>[]) {
    final hit = _find(child, text);
    if (hit != null) return hit;
  }
  return null;
}

void main() {
  testWidgets('header and two body rows draw as a 3 by 2 table', (
    tester,
  ) async {
    await _pumpTable(tester, _basic);
    expect(tester.takeException(), isNull);
    final table = tester.widget<Table>(find.byType(Table));
    expect(table.children, hasLength(3));
    for (final row in table.children) {
      expect(row.children, hasLength(2));
    }
    expect(find.text('Item'), findsOneWidget);
    expect(find.text('Count'), findsOneWidget);
    expect(find.text('Apple'), findsOneWidget);
    expect(find.text('12'), findsOneWidget);
    // The raw markdown is not shown as text.
    expect(find.textContaining('---'), findsNothing);
  });

  testWidgets('the delimiter row sets each column\'s alignment', (
    tester,
  ) async {
    await _pumpTable(tester, _aligned);
    expect(tester.widget<Text>(find.text('Left')).textAlign, TextAlign.start);
    expect(
      tester.widget<Text>(find.text('Center')).textAlign,
      TextAlign.center,
    );
    expect(tester.widget<Text>(find.text('Right')).textAlign, TextAlign.end);
    expect(tester.widget<Text>(find.text('p1')).textAlign, TextAlign.start);
    expect(tester.widget<Text>(find.text('c1')).textAlign, TextAlign.center);
    expect(tester.widget<Text>(find.text('r1')).textAlign, TextAlign.end);
  });

  testWidgets('cells show bold, italic, code and links; the rest stays plain', (
    tester,
  ) async {
    await _pumpTable(tester, _marks);
    expect(tester.takeException(), isNull);

    expect(_leaf(tester, 'Bold').style?.fontWeight, FontWeight.w700);
    expect(_leaf(tester, 'Italic').style?.fontStyle, FontStyle.italic);

    final code = _leaf(tester, 'a | b').style;
    expect(code?.backgroundColor, AppColors.light.inlineCodeBg);
    expect(_leaf(tester, 'x|y').style?.backgroundColor, AppColors.light.inlineCodeBg);

    final link = _leaf(tester, 'site').style;
    expect(link?.decoration, TextDecoration.underline);
    expect(link?.color, AppColors.light.accentText);

    // Unmatched markers and a snake_case word are plain text.
    expect(find.text('snake_case and *unclosed'), findsOneWidget);
    expect(_leaf(tester, 'snake_case and *unclosed').style?.fontStyle, isNull);

    // The escaped pipe is one cell, so the row still has two columns.
    final table = tester.widget<Table>(find.byType(Table));
    for (final row in table.children) {
      expect(row.children, hasLength(2));
    }
  });

  testWidgets('a wide table scrolls sideways in a narrow block', (
    tester,
  ) async {
    await _pumpTable(tester, _wide, width: 320);
    expect(tester.takeException(), isNull);
    final scroll = find.descendant(
      of: find.byKey(const ValueKey('table-scroll-table-1')),
      matching: find.byType(Scrollable),
    );
    expect(scroll, findsOneWidget);
    expect(tester.state<ScrollableState>(scroll).position.maxScrollExtent, greaterThan(0));
    expect(find.text('Column four'), findsOneWidget);
  });

  testWidgets('a table that fits does not scroll', (tester) async {
    await _pumpTable(tester, _basic, width: 700);
    final scroll = find.descendant(
      of: find.byKey(const ValueKey('table-scroll-table-1')),
      matching: find.byType(Scrollable),
    );
    expect(tester.state<ScrollableState>(scroll).position.maxScrollExtent, 0);
  });

  testWidgets('a link opens its http or mailto target on tap', (tester) async {
    final opened = <Uri>[];
    final before = openTableLink;
    openTableLink = opened.add;
    addTearDown(() => openTableLink = before);
    await _pumpTable(tester, _marks);
    await tester.tapAt(tester.getTopLeft(find.text('site')) + const Offset(4, 4));
    await tester.pump();
    await tester.tapAt(tester.getTopLeft(find.text('write')) + const Offset(4, 4));
    await tester.pump();
    expect(opened, [Uri.parse('https://x.test'), Uri.parse('mailto:me@x.test')]);
  });

  testWidgets('a relative file link is plain text and does not open', (
    tester,
  ) async {
    final opened = <Uri>[];
    final before = openTableLink;
    openTableLink = opened.add;
    addTearDown(() => openTableLink = before);
    await _pumpTable(tester, _marks);
    final file = _leaf(tester, 'notes');
    expect(file.style?.decoration, isNot(TextDecoration.underline));
    expect(file.recognizer, isNull);
    await tester.tapAt(tester.getTopLeft(find.text('notes')) + const Offset(4, 4));
    await tester.pump();
    expect(opened, isEmpty);
  });

  testWidgets('link recognizers can be torn down with the table', (tester) async {
    await _pumpTable(tester, _marks);
    await tester.pumpWidget(_app(const SizedBox()));
    expect(tester.takeException(), isNull);
  });

  testWidgets('a resolved page link opens its page; a wiki alias shows as its label', (
    tester,
  ) async {
    final opened = <Uri>[];
    final before = openTableLink;
    openTableLink = opened.add;
    addTearDown(() => openTableLink = before);
    final host = _Host();
    await _pumpTable(
      tester,
      _pageLinks,
      host: host,
      marks: [
        ..._pageMarks(_pageLinks, 'plan', 'plan.md', 'page-plan'),
        ..._pageMarks(_pageLinks, 'Highway', 'Road', 'page-road'),
      ],
    );
    expect(tester.takeException(), isNull);
    expect(find.text('Highway'), findsOneWidget);
    expect(find.textContaining('[[Road'), findsNothing);
    expect(_leaf(tester, 'plan').style?.decoration, TextDecoration.underline);
    expect(_leaf(tester, 'plan').recognizer, isNotNull);

    await tester.tapAt(tester.getTopLeft(find.text('plan')) + const Offset(4, 4));
    await tester.pump();
    await tester.tapAt(tester.getTopLeft(find.text('Highway')) + const Offset(4, 4));
    await tester.pump();
    expect(host.opened, ['page-plan', 'page-road']);
    expect(opened, isEmpty);
  });

  testWidgets('an unresolved wiki link stays as typed and does not open', (
    tester,
  ) async {
    final host = _Host();
    await _pumpTable(
      tester,
      _pageLinks,
      host: host,
      marks: _pageMarks(_pageLinks, 'plan', 'plan.md', 'page-plan'),
    );
    expect(tester.takeException(), isNull);
    expect(find.text('[[Missing]]'), findsOneWidget);
    expect(_leaf(tester, '[[Missing]]').recognizer, isNull);
    expect(_leaf(tester, '[[Missing]]').style?.decoration, isNot(TextDecoration.underline));
    // A relative link with no page in the marks is plain text too.
    expect(_leaf(tester, 'gone').recognizer, isNull);

    await tester.tapAt(tester.getTopLeft(find.text('[[Missing]]')) + const Offset(4, 4));
    await tester.pump();
    expect(host.opened, isEmpty);
  });

  test('only http, https with a host, and mailto are opened', () {
    expect(safeLinkUri('https://x.test/a'), isNotNull);
    expect(safeLinkUri('http://x.test'), isNotNull);
    expect(safeLinkUri('mailto:me@x.test'), isNotNull);
    expect(safeLinkUri('notes/a.md'), isNull);
    expect(safeLinkUri('https://'), isNull);
    expect(safeLinkUri('javascript:alert(1)'), isNull);
  });

  testWidgets('text that is not a table shows as plain text', (tester) async {
    await _pumpTable(tester, 'Just words | with a pipe');
    expect(find.byType(Table), findsNothing);
    expect(find.text('Just words | with a pipe'), findsOneWidget);
  });

  testWidgets('the code block labels its box with the target, or Plain text', (
    tester,
  ) async {
    Widget code(String? target) => Builder(
      builder: (context) => _render(
        context,
        Node(id: 'code-1', kind: Kinds.code, text: 'x = 1', target: target),
        codeBlock,
      ),
    );
    await tester.pumpWidget(_app(_sized(400, code(null))));
    expect(find.text('Plain text'), findsOneWidget);

    await tester.pumpWidget(_app(_sized(400, code('dart'))));
    expect(find.text('dart'), findsOneWidget);
    expect(find.text('Plain text'), findsNothing);
  });

  test('a table writes its markdown verbatim and code writes its target', () {
    final registry = BlockRegistry();
    registerBuiltinBlocks(registry);
    expect(
      nodesToMarkdown([
        Node(id: 't', kind: Kinds.table, text: _basic),
      ], registry: registry),
      _basic,
    );
    expect(
      nodesToMarkdown([
        Node(id: 'c', kind: Kinds.code, text: 'x = 1', target: 'dart'),
      ], registry: registry),
      '```dart\nx = 1\n```',
    );
    expect(
      nodesToMarkdown([
        Node(id: 'c', kind: Kinds.code, text: 'x = 1'),
      ], registry: registry),
      '```\nx = 1\n```',
    );
  });

  test('the table is a registered kind with its own codecs', () {
    final registry = BlockRegistry();
    registerBuiltinBlocks(registry);
    expect(registry.has(Kinds.table), isTrue);
    expect(registry.of(Kinds.table).kind, Kinds.table);
    expect(registry.incompleteCodecKinds, isNot(contains(Kinds.table)));
    expect(registry.of(Kinds.table).inMenu, isFalse);
  });

  testWidgets('an unregistered table still shows its source text', (tester) async {
    final registry = BlockRegistry();
    final node = Node(id: 'future-table', kind: Kinds.table, text: _basic);
    await tester.pumpWidget(
      _app(
        Builder(
          builder: (context) => _render(context, node, registry.of(Kinds.table)),
        ),
      ),
    );
    expect(find.text('${Kinds.table} · $_basic'), findsOneWidget);
    expect(tester.takeException(), isNull);
  });

  test('an unknown kind still gets the read-only placeholder', () {
    final registry = BlockRegistry();
    registerBuiltinBlocks(registry);
    const kind = 'future:thing';
    final type = registry.of(kind);
    expect(type.label, 'Unsupported block');
    expect(type.editable, isFalse);
    expect(type.inMenu, isFalse);
  });
}
