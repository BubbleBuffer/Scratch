import 'dart:convert';

import 'dart:async';

import 'package:flutter/widgets.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:notes_app/addons/database/create.dart' show makeDatabase;
import 'package:notes_app/addons/document/api.dart';
import 'package:notes_app/core/models.dart';
import 'package:notes_app/editor/model.dart';
import 'package:notes_app/registry/addon_registry.dart';
import 'package:notes_app/registry/block_registry.dart';
import 'package:notes_app/registry/embed_block.dart';
import 'package:notes_app/registry/note_type_registry.dart';
import 'package:notes_app/state/app_state.dart';
import 'package:notes_app/ui/embed.dart';
import 'package:notes_app/ui/embed_document.dart';
import 'package:notes_app/ui/ui.dart';

import 'harness.dart';

PageData page(
  String id, {
  String type = 'document',
  String? next,
  String text = '',
}) => PageData(
  id: id,
  title: 'Title $id',
  blocks: [
    if (text.isNotEmpty)
      FlatBlock(
        id: 'text-$id',
        kind: Kinds.paragraph,
        text: text,
        marks: const [],
        checked: false,
        depth: 0,
      ),
    if (next != null)
      FlatBlock(
        id: 'next-$id',
        kind: embedKind,
        text: '',
        target: next,
        marks: const [],
        checked: false,
        depth: 0,
      ),
  ],
  properties: const [],
  pageType: type,
  favorite: false,
  inTrash: false,
  createdMs: 0,
  updatedMs: 0,
  loadUs: 0,
  prefs: const PagePrefsData(smallText: false, fullWidth: false, locked: false),
);

Widget frame(Widget child) => MediaQuery(
  data: const MediaQueryData(size: Size(900, 900)),
  child: Directionality(
    textDirection: TextDirection.ltr,
    child: AppTheme(
      colors: AppColors.light,
      formFactor: FormFactor.desktop,
      child: DefaultTextStyle(
        style: const TextStyle(fontSize: 16),
        child: Align(
          alignment: Alignment.topLeft,
          child: SizedBox(width: 700, child: child),
        ),
      ),
    ),
  ),
);

AddonContext documents() {
  final ctx = AddonContext();
  bootAddons(const [DocumentAddon()], ctx);
  ctx.blockKinds.register(
    const BlockType(kind: Kinds.paragraph, label: 'Paragraph'),
  );
  return ctx;
}

void main() {
  testWidgets('retargeting hides the previous note while the new body loads', (
    tester,
  ) async {
    final ctx = documents();
    addTearDown(ctx.noteTypes.dispose);
    final second = Completer<EmbedNoteData?>();
    Future<EmbedNoteData?> load(String id) => id == 'one'
        ? Future.value(EmbedNoteData.fromPage(page(id, text: 'Old body')))
        : second.future;
    Widget embed(String id) => frame(
      EmbedNote(
        key: const ValueKey('retarget'),
        targetId: id,
        rootPageId: 'root',
        addons: ctx,
        loader: load,
      ),
    );
    await tester.pumpWidget(embed('one'));
    await tester.pump();
    expect(find.text('Old body', findRichText: true), findsOneWidget);
    await tester.pumpWidget(embed('two'));
    expect(find.text('Old body', findRichText: true), findsNothing);
    expect(find.text('Loading note…'), findsOneWidget);
    second.complete(EmbedNoteData.fromPage(page('two', text: 'New body')));
    await tester.pump();
    await tester.pump();
    expect(find.text('New body', findRichText: true), findsOneWidget);
  });

  test('embed belongs to the kit without any installed add-on', () {
    final ctx = AddonContext();
    addTearDown(ctx.noteTypes.dispose);
    bootAddons(const [], ctx);
    expect(ctx.installedIds, isEmpty);
    expect(ctx.blockKinds.has(embedKind), isTrue);
    expect(ctx.blockKinds.incompleteCodecKinds, isEmpty);
  });

  test('invalid dependency graph cannot partially register kit blocks', () {
    final ctx = AddonContext();
    addTearDown(ctx.noteTypes.dispose);
    expect(
      () => bootAddons(const [_MissingDependency()], ctx),
      throwsStateError,
    );
    expect(ctx.blockKinds.all, isEmpty);
  });

  test(
    'embed codecs preserve only target metadata and never use block text',
    () {
      final node = Node(
        id: 'embed',
        kind: embedKind,
        target: 'target-雪-->',
        text: 'Obsolete cached title',
      );
      embedTitleResolver = (id) => id == node.target ? 'Current title' : null;
      addTearDown(() => embedTitleResolver = null);
      final lines = embedBlockCodecs.markdown.write(node, node.text);
      expect(lines.single, startsWith('> Current title <!-- block='));
      expect(lines.single, isNot(contains('Obsolete cached title')));
      final hex = RegExp(r'block=([0-9a-f]+)')
          .firstMatch(lines.single)!
          .group(1)!;
      final payload = jsonDecode(
        utf8.decode([
          for (var i = 0; i < hex.length; i += 2)
            int.parse(hex.substring(i, i + 2), radix: 16),
        ]),
      );
      expect(payload, {
        'kind': embedKind,
        'target': 'target-雪-->',
        'lines': 1,
        'checked': false,
      });
      expect(embedBlockCodecs.plainText.line(node), 'Current title');
      expect(node.text, 'Obsolete cached title');
      expect(node.target, 'target-雪-->');
    },
  );

  testWidgets('registered document embed renders its real read-only blocks', (
    tester,
  ) async {
    final ctx = documents();
    addTearDown(ctx.noteTypes.dispose);
    await tester.pumpWidget(
      frame(
        EmbedNote(
          targetId: 'B',
          rootPageId: 'A',
          addons: ctx,
          loader: (id) async =>
              EmbedNoteData.fromPage(page(id, text: 'Real body')),
        ),
      ),
    );
    await tester.pump();
    expect(find.byType(EmbedDocument), findsOneWidget);
    expect(find.text('Title B'), findsOneWidget);
    expect(find.text('Real body', findRichText: true), findsOneWidget);
    expect(find.byType(EditableText), findsNothing);
  });

  testWidgets('empty embed delegates to the existing block picker', (
    tester,
  ) async {
    final ctx = documents();
    addTearDown(ctx.noteTypes.dispose);
    final host = _PickerHost();
    final node = Node(id: 'empty', kind: embedKind);
    await tester.pumpWidget(
      frame(
        Builder(
          builder: (context) => ctx.blockKinds.of(embedKind).body!(
            BlockContext(
              context: context,
              node: node,
              host: host,
              style: const TextStyle(fontSize: 16),
              lineHeight: 24,
            ),
          ),
        ),
      ),
    );
    await tester.tap(find.byKey(const ValueKey('empty-embed-empty')));
    expect(host.opens, 1);
    expect(host.selectedNode, same(node));
    expect(node.target, isNull);
    expect(node.text, isEmpty);
  });

  testWidgets('document A to B to A cycle ends as a title chip', (
    tester,
  ) async {
    final ctx = documents();
    addTearDown(ctx.noteTypes.dispose);
    final pages = {'A': page('A', next: 'B'), 'B': page('B', next: 'A')};
    await tester.pumpWidget(
      frame(
        EmbedNote(
          targetId: 'B',
          rootPageId: 'A',
          addons: ctx,
          loader: (id) async => EmbedNoteData.fromPage(pages[id]!),
        ),
      ),
    );
    await tester.pump();
    await tester.pump();
    expect(find.byType(EmbedDocument), findsOneWidget);
    expect(find.byKey(const ValueKey('embed-link-next-B')), findsOneWidget);
    expect(find.text('Title A'), findsOneWidget);
  });

  testWidgets('four nested document bodies render then depth cap draws chip', (
    tester,
  ) async {
    final ctx = documents();
    addTearDown(ctx.noteTypes.dispose);
    final pages = {
      for (var i = 1; i <= 5; i++)
        '$i': page('$i', next: i == 5 ? null : '${i + 1}', text: 'Body $i'),
    };
    await tester.pumpWidget(
      frame(
        EmbedNote(
          targetId: '1',
          rootPageId: 'root',
          addons: ctx,
          loader: (id) async => EmbedNoteData.fromPage(pages[id]!),
        ),
      ),
    );
    for (var i = 0; i < 7; i++) {
      await tester.pump();
    }
    expect(find.byType(EmbedDocument), findsNWidgets(maxEmbedDepth));
    expect(find.text('Body 4', findRichText: true), findsOneWidget);
    expect(find.text('Body 5', findRichText: true), findsNothing);
    expect(find.byKey(const ValueKey('embed-link-next-4')), findsOneWidget);
    expect(find.text('Title 5'), findsOneWidget);
  });

  testWidgets('unknown note type retains title and actual block fallback', (
    tester,
  ) async {
    final ctx = documents();
    addTearDown(ctx.noteTypes.dispose);
    await tester.pumpWidget(
      frame(
        EmbedNote(
          targetId: 'custom',
          rootPageId: 'A',
          addons: ctx,
          loader: (id) async => EmbedNoteData.fromPage(
            page(id, type: 'mod:example', text: 'Preserved fallback'),
          ),
        ),
      ),
    );
    await tester.pump();
    expect(find.text('Title custom'), findsOneWidget);
    expect(find.text('Unsupported note type (mod:example)'), findsOneWidget);
    expect(find.text('Preserved fallback', findRichText: true), findsOneWidget);
  });

  testWidgets('disabled external type retains indexed title and summary', (
    tester,
  ) async {
    final ctx = documents();
    addTearDown(ctx.noteTypes.dispose);
    await tester.pumpWidget(
      frame(
        EmbedNote(
          targetId: 'chat:session',
          rootPageId: 'A',
          addons: ctx,
          loader: (id) async => EmbedNoteData.fromHit(
            Hit(
              id: id,
              title: 'Stored chat title',
              snippet: 'Indexed summary',
              source: 'agent',
              pageType: 'chat',
              fuzzy: false,
            ),
          ),
        ),
      ),
    );
    await tester.pump();
    expect(ctx.noteTypes.of('chat'), isNull);
    expect(find.text('Stored chat title'), findsOneWidget);
    expect(find.text('Unsupported note type (chat)'), findsOneWidget);
    expect(find.text('Indexed summary'), findsOneWidget);
  });

  testWidgets('a registered type without an embed draws its card', (
    tester,
  ) async {
    final ctx = documents();
    addTearDown(ctx.noteTypes.dispose);
    Widget card(BuildContext context, NoteArgs args) =>
        Text('Card ${args.title}');
    Widget other(BuildContext context, NoteArgs args) =>
        const Text('Other size');
    ctx.registerNoteType(
      NoteType(
        id: 'card-only',
        label: 'Card only',
        chip: other,
        card: card,
        fullPage: other,
        usesEditor: false,
      ),
    );
    await tester.pumpWidget(
      frame(
        EmbedNote(
          targetId: 'card',
          rootPageId: 'A',
          addons: ctx,
          loader: (id) async =>
              EmbedNoteData.fromPage(page(id, type: 'card-only')),
        ),
      ),
    );
    await tester.pump();
    expect(find.text('Card Title card'), findsOneWidget);
    expect(find.text('Other size'), findsNothing);
  });

  setUpAll(initCore);

  appTest('editing a target refreshes the embedded document title and body', (
    tester,
  ) async {
    final tv = await pumpApp(tester, mobile: false, empty: true);
    final id = await core(tester, (v) => v.createPage(title: 'Old title'));
    await core(
      tester,
      (v) => v.apply(id, [
        const BOp(
          op: OpKind.insert,
          id: 'body',
          kind: Kinds.paragraph,
          text: 'Old contents',
          marks: [],
          checked: false,
        ),
      ]),
    );
    await tester.pumpWidget(
      AppScope(
        state: tv.app,
        child: frame(EmbedNote(targetId: id, rootPageId: 'host')),
      ),
    );
    await settle(tester);
    expect(find.text('Old contents', findRichText: true), findsOneWidget);
    await core(tester, (v) => v.renamePage(id, 'New title'));
    await core(
      tester,
      (v) => v.apply(id, [
        const BOp(
          op: OpKind.setText,
          id: 'body',
          kind: Kinds.paragraph,
          text: 'New contents',
          marks: [],
          checked: false,
        ),
      ]),
    );
    await settle(tester);
    expect(find.text('Old title'), findsNothing);
    expect(find.text('Old contents', findRichText: true), findsNothing);
    expect(find.text('New title'), findsOneWidget);
    expect(find.text('New contents', findRichText: true), findsOneWidget);
  });

  appTest('vault document recursion detects persisted A to B to A cycle', (
    tester,
  ) async {
    final tv = await pumpApp(tester, mobile: false, empty: true);
    final ids = (await tester.runAsync(() async {
      final a = await tv.vault.createPage(title: 'Document A');
      final b = await tv.vault.createPage(title: 'Document B');
      Future<void> embed(String host, String target) => tv.vault.apply(host, [
        BOp(
          op: OpKind.insert,
          id: 'embed-$host',
          kind: embedKind,
          text: '',
          marks: const [],
          checked: false,
        ),
        BOp(
          op: OpKind.setTarget,
          id: 'embed-$host',
          kind: embedKind,
          text: '',
          target: target,
          marks: const [],
          checked: false,
        ),
      ]);
      await embed(a, b);
      await embed(b, a);
      return [a, b];
    }))!;
    await tester.pumpWidget(
      AppScope(
        state: tv.app,
        child: frame(EmbedNote(targetId: ids[1], rootPageId: ids[0])),
      ),
    );
    await settle(tester);
    expect(find.byType(EmbedDocument), findsOneWidget);
    expect(find.byKey(ValueKey('embed-link-embed-${ids[1]}')), findsOneWidget);
    expect(find.text('Document A'), findsOneWidget);
    expect(find.text('Document B'), findsOneWidget);
  });

  appTest(
    'disabled agent embeds resolve exact indexed note title and fallback',
    (tester) async {
      final ctx = documents();
      addTearDown(ctx.noteTypes.dispose);
      final tv = await pumpApp(tester, mobile: false, empty: true);
      await tester.runAsync(
        () => tv.vault.run(
          (v) => v.upsertSearchRow(
            source: 'agent',
            row: const ExternalSearchRow(
              id: 'chat:fixture-session',
              title: 'Live indexed chat title',
              body: 'Saved chapter and section summary',
              pageType: 'chat',
              target: '{"session_id":"fixture-session","level":"chat"}',
              updatedMs: 1,
            ),
          ),
          writes: true,
        ),
      );
      await tester.pumpWidget(
        AppScope(
          state: tv.app,
          child: frame(
            EmbedNote(
              targetId: 'chat:fixture-session',
              rootPageId: 'host',
              addons: ctx,
            ),
          ),
        ),
      );
      await settle(tester);
      expect(ctx.noteTypes.of('chat'), isNull);
      expect(find.text('Live indexed chat title'), findsOneWidget);
      expect(find.text('Unsupported note type (chat)'), findsOneWidget);
      expect(find.text('Saved chapter and section summary'), findsOneWidget);
      expect(
        embedTitleOf(tv.vault, 'chat:fixture-session'),
        'Live indexed chat title',
      );
    },
  );

  appTest('database embed dispatch keeps the exact inline DatabaseView', (
    tester,
  ) async {
    final tv = await pumpApp(tester, mobile: false, empty: true);
    final id = (await tester.runAsync(() async {
      final id = await tv.vault.createPage(title: 'Embedded database');
      await makeDatabase(tv.vault, id);
      return id;
    }))!;
    await tester.pumpWidget(
      AppScope(
        state: tv.app,
        child: frame(EmbedNote(targetId: id, rootPageId: 'host')),
      ),
    );
    await settle(tester);
    // The target type supplies its original frame/key instead of a kit card.
    expect(find.byKey(ValueKey('inline-db-$id')), findsOneWidget);
    expect(find.byType(EmbedDocument), findsNothing);
    expect(find.text('Embedded database'), findsOneWidget);
  });
}

class _MissingDependency extends Addon {
  const _MissingDependency();
  @override
  String get id => 'missing-test';
  @override
  List<String> get dependsOn => const ['not-installed'];
  @override
  void register(AddonContext ctx) {}
}

class _PickerHost implements BlockHost {
  int opens = 0;
  Node? selectedNode;
  @override
  String get pageId => 'host';
  @override
  void openBlockPicker(Node n, BuildContext anchor) {
    opens++;
    selectedNode = n;
  }

  @override
  dynamic noSuchMethod(Invocation invocation) =>
      throw UnsupportedError('Unexpected host call ${invocation.memberName}');
}
