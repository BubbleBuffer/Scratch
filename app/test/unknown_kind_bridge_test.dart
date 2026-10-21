// Unknown core kinds arrive in Dart unchanged and use the registry placeholder.
import 'dart:io';

import 'package:flutter_test/flutter_test.dart';
import 'package:notes_app/core/models.dart';
import 'package:notes_app/core/vault.dart';
import 'package:notes_app/registry/block_registry.dart';

import 'harness.dart';
import 'sync_helpers.dart' show real;

const unknownKind = 'mod:future kind 雪 -->';
const fallback = 'Future block fallback';
const unknownTarget = '{"target":"spaces --> 雪"}';

List<BOp> unknownOps(String id) => [
  BOp(
    op: OpKind.insert,
    id: id,
    kind: unknownKind,
    text: fallback,
    marks: const [],
    checked: false,
  ),
  BOp(
    op: OpKind.setTarget,
    id: id,
    kind: unknownKind,
    text: '',
    marks: const [],
    checked: false,
    target: unknownTarget,
  ),
];

void main() {
  setUpAll(initCore);

  test(
    'query card and chip round-trip through a native bridge reopen',
    () async {
      final dir = Directory.systemTemp.createTempSync('notes_query_bridge_');
      final path = '${dir.path}${Platform.pathSeparator}vault';
      const spec = 'tag:work in:[[stable-page-id]] title:"雪 & <open>"';
      const label = '🐈 4 open #work tasks';
      final cardId = newId(), chipId = newId();
      Vault? opened;
      try {
        var vault = await Vault.open(path);
        opened = vault;
        final pageId = await vault.createPage(title: 'Stored queries');
        await vault.apply(pageId, [
          BOp(
            op: OpKind.insert,
            id: cardId,
            kind: 'query',
            text: spec,
            marks: const [],
            checked: false,
          ),
          BOp(
            op: OpKind.insert,
            id: chipId,
            kind: 'paragraph',
            text: label,
            marks: [
              BMark(
                start: 3,
                end: label.length,
                kind: BMarkKind.query,
                url: spec,
              ),
            ],
            checked: false,
          ),
        ]);
        await vault.flush();
        vault.dispose();
        opened = null;
        vault = await Vault.open(path);
        opened = vault;
        final blocks = (await vault.loadPage(pageId)).blocks;
        final card = blocks.singleWhere((b) => b.id == cardId);
        expect(card.kind, 'query');
        expect(card.text, spec);
        expect(card.marks, isEmpty);
        final chip = blocks.singleWhere((b) => b.id == chipId);
        expect(chip.text, label);
        expect(chip.marks, hasLength(1));
        expect(chip.marks.single.kind, BMarkKind.query);
        expect(chip.marks.single.url, spec);
        expect(chip.marks.single.start, 3);
        expect(chip.marks.single.end, label.length);
      } finally {
        opened?.dispose();
        await dir.delete(recursive: true);
      }
    },
  );

  test(
    'unknown kind, target and text survive a native bridge reopen',
    () async {
      final dir = Directory.systemTemp.createTempSync('notes_unknown_bridge_');
      final path = '${dir.path}${Platform.pathSeparator}vault';
      Vault? opened;
      try {
        var vault = await Vault.open(path);
        opened = vault;
        final pageId = await vault.createPage(title: 'Unknown kind');
        final id = newId();
        await vault.apply(pageId, unknownOps(id));
        await vault.flush();
        vault.dispose();
        opened = null;
        vault = await Vault.open(path);
        opened = vault;
        final block = (await vault.loadPage(pageId)).blocks.single;
        expect(block.kind, unknownKind);
        expect(block.target, unknownTarget);
        expect(block.text, fallback);
        final type = blockRegistry.of(block.kind);
        expect(type.kind, unknownKind);
        expect(type.editable, isFalse);
        expect(type.inMenu, isFalse);
      } finally {
        opened?.dispose();
        await dir.delete(recursive: true);
      }
    },
  );

  appTest('raw unknown kind renders its registry placeholder in the editor', (
    tester,
  ) async {
    final tv = await pumpApp(tester, mobile: false);
    {
      final page = tv.pages['Garden plan']!;
      await real(tester, () => tv.vault.apply(page, unknownOps(newId())));
      await real(tester, () => tv.vault.flush());
      await tapAndSettle(tester, find.text('Garden plan').last);
      await settle(tester);
      expect(
        find.text('Not supported on this version of Scratch'),
        findsOneWidget,
      );
      expect(find.text('$unknownKind · $fallback'), findsOneWidget);
      expect(tester.takeException(), isNull);
    }
  });
}
