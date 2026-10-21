// Shell and editor polish on the real core: home's "All pages" without
// database rows, sidebar favorites that expand, the row highlight while its
// menu is open, mention icons, bookmark previews, columns made by dragging
// and the history preview.

import 'dart:convert';
import 'dart:io';

import 'package:flutter/gestures.dart';
import 'package:flutter/services.dart';
import 'package:flutter/widgets.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:notes_app/core/file_dialogs.dart';
import 'package:notes_app/core/link_previews.dart';
import 'package:notes_app/editor/rich_controller.dart';
import 'package:notes_app/editor/block_pickers.dart';
import 'package:notes_app/features/import/import_panel.dart';
import 'package:notes_app/features/page/cover_picker.dart';
import 'package:notes_app/shell/home.dart';
import 'package:notes_app/ui/ui.dart' show Hover;

import 'harness.dart';
import 'pages_fixture.dart';

Finder key(String k) => find.byKey(ValueKey(k));
BuildContext homeContext(WidgetTester tester) => tester.element(find.byType(HomeScreen));
String resultOf(WidgetTester tester, String k) =>
    key(k).evaluate().isEmpty ? '' : (tester.widget<Text>(key(k)).data ?? '');

Directory tempDir(String name) {
  final d = Directory.systemTemp.createTempSync(name);
  addTearDown(() {
    try {
      d.deleteSync(recursive: true);
    } catch (_) {}
  });
  return d;
}

String join(String a, String b) => '$a${Platform.pathSeparator}$b';

void main() {
  setUpAll(initCore);

  group('home', () {
    appTest('All pages leaves out database rows', (tester) async {
      final s = await pumpApp(tester, mobile: false);
      final (db, rows) = await core(tester, (v) async {
        final id = await v.createPage(title: 'Reading list');
        await v.run((x) => x.makeCollection(page: id), writes: true);
        final rows = [
          for (final t in ['Dune', 'Emma', 'Ulysses']) await v.createPage(parent: id, title: t),
        ];
        return (id, rows);
      });
      final pages = s.app.pages;
      await tester.runAsync(pages.reload);
      await settle(tester);
      expect(pages.docComplete, isTrue);
      final ids = [for (var i = 0; i < pages.docCount; i++) pages.docAt(i)!.id];
      expect(ids, contains(db));
      expect(ids, contains(s.pages['Soups']));
      for (final r in rows) {
        expect(ids, isNot(contains(r)));
        expect(key('row-$r'), findsNothing);
      }
      // Every other page is there: the fixture, the showcase and the database.
      expect(pages.docCount, pages.count - rows.length);
      expect(find.text('${pages.docCount}'), findsOneWidget);
    });
  });

  group('file dialogs', () {
    appTest('import: Choose picks a folder and imports it', (tester) async {
      await pumpApp(tester, mobile: false);
      final dir = tempDir('notes_import_');
      File(join(dir.path, 'Picked note.md')).writeAsStringSync('# Picked note\n\nFrom the dialog.\n');
      dialogs.folder = dir.path;
      showImport(homeContext(tester));
      await settle(tester);
      await tester.tap(key('import-choose'));
      await settleUntil(tester, () => resultOf(tester, 'import-result').startsWith('Imported'));
      expect(dialogs.calls, ['folder']);
      expect(resultOf(tester, 'import-result'), startsWith('Imported 1 page'));
      expect(tester.widget<EditableText>(find.descendant(of: key('import-path'), matching: find.byType(EditableText)).first).controller.text,
          dir.path);
    });

    appTest('import: without folders (Android) Choose picks Markdown files', (tester) async {
      await pumpApp(tester, mobile: true);
      FileDialogs.instance = FakeFileDialogs(folders: false)
        ..files = [
          pickedBytes('One.md', utf8.encode('# One\n\nFirst.\n')),
          pickedBytes('Two.md', utf8.encode('Second.\n')),
        ];
      showImport(homeContext(tester));
      await settle(tester);
      await tester.tap(key('import-choose'));
      await settleUntil(tester, () => resultOf(tester, 'import-result').startsWith('Imported'));
      expect(dialogs.calls, ['files']);
      expect(resultOf(tester, 'import-result'), startsWith('Imported 2 pages'));
    });

    appTest('export: Choose saves where the dialog says', (tester) async {
      final s = await pumpApp(tester, mobile: false);
      final dir = tempDir('notes_export_');
      final path = join(dir.path, 'garden.md');
      dialogs.savePath = path;
      showExport(homeContext(tester), s.pages['Garden plan']!);
      await settle(tester);
      await tester.tap(key('export-choose'));
      await settleUntil(tester, () => resultOf(tester, 'export-result').startsWith('Exported'));
      expect(dialogs.calls, ['save:Garden plan.md']);
      final expected = await core(tester, (v) => pageToMarkdown(v, s.pages['Garden plan']!));
      expect(File(path).readAsStringSync(), expected);
      expect(resultOf(tester, 'export-result'), 'Exported to $path');
      // Cancelling changes nothing.
      dialogs.savePath = null;
      await tester.tap(key('export-choose'));
      await settle(tester);
      expect(resultOf(tester, 'export-result'), 'Exported to $path');
    });

    appTest('attachments and covers come from the dialog, by path or by bytes', (tester) async {
      final s = await pumpApp(tester, mobile: false);
      final dir = tempDir('notes_attach_');
      final file = File(join(dir.path, 'notes.txt'))..writeAsStringSync('hello');
      dialogs.files = [PickedFile('notes.txt', path: file.path)];
      final picked = pickAttachment(homeContext(tester), const Rect.fromLTWH(400, 200, 10, 10), image: false);
      await settle(tester);
      expect(key('attachment-picker'), findsOneWidget);
      await tester.tap(key('picker-submit'));
      await settleUntil(tester, () => key('attachment-picker').evaluate().isEmpty);
      final id = await picked;
      expect(id, isNotNull);
      final info = await core(tester, (v) => v.run((x) => x.attachmentInfo(id: id!)));
      expect(info.name, 'notes.txt');
      expect(info.size.toInt(), 5);

      // A cover from bytes only.
      dialogs.files = [pickedBytes('sky.png', List.filled(64, 1))];
      final cover = showCoverPicker(homeContext(tester),
          pageId: s.pages['Garden plan']!, anchor: const Rect.fromLTWH(400, 200, 10, 10));
      await settle(tester);
      await tapAndSettle(tester, key('cover-tab-1'));
      await tester.tap(key('cover-upload'));
      await settleUntil(tester, () => key('cover-upload').evaluate().isEmpty);
      final cid = await cover;
      final cinfo = await core(tester, (v) => v.run((x) => x.attachmentInfo(id: cid!)));
      expect(cinfo.name, 'sky.png');
      expect(dialogs.calls, ['files', 'files']);
    });

    appTest('settings: Choose picks the vault folder', (tester) async {
      await pumpApp(tester, mobile: false);
      final dir = tempDir('notes_vault_');
      dialogs.folder = dir.path;
      await tapAndSettle(tester, key('sidebar-settings'));
      await tapAndSettle(tester, key('settings-nav-vault'));
      await tapAndSettle(tester, key('vault-choose'));
      expect(dialogs.calls, ['folder']);
      expect(find.textContaining('Scratch opens ${dir.path} the next time it starts.'), findsOneWidget);
    });
  });

  group('sidebar', () {
    appTest('favorites expand like the tree, apart from it', (tester) async {
      final s = await pumpApp(tester, mobile: false);
      final recipes = s.pages['Recipes']!, soups = s.pages['Soups']!, breads = s.pages['Breads']!;
      await core(tester, (v) => v.setFavorite(recipes, true));
      await settleUntil(tester, () => key('fav-$recipes').evaluate().isNotEmpty);
      expect(key('fav-$recipes-$soups'), findsNothing);

      await tapAndSettle(tester, key('fav-toggle-$recipes'));
      expect(key('fav-$recipes-$soups'), findsOneWidget);
      expect(key('fav-$recipes-$breads'), findsOneWidget);
      // Nested rows sit below their favorite, indented, and above Pages.
      expect(tester.getTopLeft(key('fav-$recipes-$soups')).dy, greaterThan(tester.getTopLeft(key('fav-$recipes')).dy));
      expect(tester.getTopLeft(key('fav-$recipes-$soups')).dy, lessThan(tester.getTopLeft(find.descendant(of: key('sidebar'), matching: find.text('Pages'))).dy));
      // The tree keeps its own state.
      expect(key('tree-$soups'), findsNothing);
      expect(s.app.ui.favExpanded, {recipes});
      expect(s.app.ui.expanded, isNot(contains(recipes)));

      // A nested row opens its page; Soups has no subpages, so no chevron.
      expect(key('fav-toggle-$soups'), findsNothing);
      expect(key('fav-empty-$recipes-$soups'), findsNothing);
      await tapAndSettle(tester, key('fav-$recipes-$soups'));
      expect(tester.widget<EditableText>(key('page-title')).controller.text, 'Soups');

      await tapAndSettle(tester, key('fav-toggle-$recipes'));
      expect(key('fav-$recipes-$soups'), findsNothing);
      expect(s.app.ui.favExpanded, isEmpty);
    });

    appTest('a row keeps its highlight and buttons while its menu is open', (tester) async {
      final s = await pumpApp(tester, mobile: false);
      final budget = s.pages['Budget']!;
      Hover rowHover() => tester.widget<Hover>(
          find.descendant(of: key('tree-$budget'), matching: find.byType(Hover)).first);
      final g = await tester.createGesture(kind: PointerDeviceKind.mouse);
      await g.addPointer(location: tester.getCenter(key('tree-$budget')));
      await settle(tester);
      await tapAndSettle(tester, key('tree-more-$budget'));
      expect(key('tree-menu-rename'), findsOneWidget);
      // The pointer goes into the menu.
      await g.moveTo(tester.getCenter(key('tree-menu-rename')));
      await settle(tester);
      expect(key('tree-more-$budget'), findsOneWidget);
      expect(rowHover().selected, isTrue);

      // Closing the menu with the pointer elsewhere drops it.
      await g.moveTo(const Offset(900, 700));
      await tester.sendKeyEvent(LogicalKeyboardKey.escape);
      await settle(tester);
      expect(key('tree-menu-rename'), findsNothing);
      expect(key('tree-more-$budget'), findsNothing);
      expect(rowHover().selected, isFalse);
      await g.removePointer();
    });
  });

  group('editor', () {
    appTest('a page mention shows the page icon in front of it', (tester) async {
      final s = await pumpApp(tester, mobile: false);
      await createRichPage(tester, s);
      final recipes = s.pages['Recipes']!;
      await core(tester, (v) => v.setIcon(recipes, '🍲'));
      await openByTitle(tester, richTitle);
      final f = editableWith('Book the ferry with Recipes in mind before @Friday, and pack light.');
      expect(f, findsOneWidget);
      final ctl = tester.widget<EditableText>(f).controller as RichController;
      expect(ctl.mentionGap, greaterThan(10));
      await settle(tester);
      expect(ctl.mentionIcon!(recipes), '🍲');
      // The character before the mention leaves room for the icon...
      final at = ctl.text.indexOf('Recipes');
      final re = tester.state<EditableTextState>(f).renderEditable;
      final before = re.getBoxesForSelection(TextSelection(baseOffset: at - 1, extentOffset: at)).first;
      final first = re.getBoxesForSelection(TextSelection(baseOffset: at, extentOffset: at + 1)).first;
      expect(first.left - before.left, greaterThan(ctl.mentionGap));
      // ...and the other characters keep their width.
      final w = re.getBoxesForSelection(TextSelection(baseOffset: at - 2, extentOffset: at - 1)).first;
      expect(w.right - w.left, lessThan(ctl.mentionGap));
      // Without mentions there is no gap.
      final plain = markedSpan('a b', const [], ctl.colors, null, mentionGap: 20);
      expect(plain.toPlainText(), 'a b');
    });

    appTest('a bookmark shows the fetched title, description and favicon', (tester) async {
      final s = await pumpApp(tester, mobile: false);
      const url = 'https://www.example.com/ferries/timetable';
      final png = await tester.runAsync(() => testPng(const [Color(0xFF3366CC)], w: 16, h: 16));
      fakeLinks[url] = LinkPreview(
          title: 'Ferry timetable', description: 'Every sailing, every day.', icon: Uint8List.fromList(png!));
      await createRichPage(tester, s);
      await openByTitle(tester, richTitle);
      await tester.dragUntilVisible(key('bookmark-r21'), find.byType(CustomScrollView).last, const Offset(0, -200));
      await settle(tester);
      expect(find.text('Ferry timetable'), findsOneWidget);
      expect(find.text('Every sailing, every day.'), findsOneWidget);
      expect(key('bookmark-favicon-$url'), findsOneWidget);
      expect(fetchedLinks.where((u) => u == url), hasLength(1));
    });

    appTest('a bookmark without a preview (offline) shows the site name', (tester) async {
      final s = await pumpApp(tester, mobile: false);
      await createRichPage(tester, s);
      await openByTitle(tester, richTitle);
      await tester.dragUntilVisible(key('bookmark-r21'), find.byType(CustomScrollView).last, const Offset(0, -200));
      await settle(tester);
      expect(find.text('example.com'), findsOneWidget);
      expect(find.text('https://www.example.com/ferries/timetable'), findsOneWidget);
      // Not asked again while the app runs.
      s.app.links.of('https://www.example.com/ferries/timetable');
      await settle(tester);
      expect(fetchedLinks, hasLength(1));
    });
  });

  group('columns by dragging', () {
    Future<void> dragBlock(WidgetTester tester, String id, Offset to) async {
      final g = await tester.createGesture(kind: PointerDeviceKind.mouse);
      await g.addPointer(location: Offset.zero);
      await g.moveTo(tester.getCenter(find.byType(EditableText).first));
      await settle(tester);
      final row = tester.getCenter(key('grip-$id'));
      await g.moveTo(row);
      await settle(tester);
      await g.down(row);
      await tester.pump();
      for (var i = 1; i <= 10; i++) {
        await g.moveTo(Offset.lerp(row, to, i / 10)!);
        await tester.pump();
      }
      await g.up();
      await settle(tester);
      await g.removePointer();
    }

    String shape(List<dynamic> blocks) =>
        [for (final b in blocks) if (b.depth <= 2) '${b.depth}:${b.kind == 'paragraph' || b.kind == 'heading2' ? b.id : b.kind}'].take(6).join(' ');

    appTest('a block dropped at the side of another makes columns; moving it out undoes them', (tester) async {
      final s = await pumpApp(tester, mobile: false);
      await openShowcase(tester);
      // Hover a row first so its handle shows.
      final g = await tester.createGesture(kind: PointerDeviceKind.mouse);
      await g.addPointer(location: tester.getCenter(editableWith('Heading 2')));
      await settle(tester);
      await g.removePointer();
      final first = editableStarting('This page has');
      final right = tester.getTopRight(first) + const Offset(-6, 6);
      await dragBlock(tester, 'sc04', right);
      var blocks = await blocksOf(tester, s.showcaseId);
      expect(shape(blocks), startsWith('0:columnlist 1:column 2:sc01 1:column 2:sc04 0:'));

      // A third column, on the left of the list.
      final list = editableStarting('This page has');
      await tester.pump();
      final left = Offset(tester.getTopLeft(list).dx + 4, tester.getTopLeft(list).dy + 6);
      await dragBlock(tester, 'sc03', left);
      blocks = await blocksOf(tester, s.showcaseId);
      expect(shape(blocks), startsWith('0:columnlist 1:column 2:sc03 1:column 2:sc01 1:column'));

      // Back out to the end of the page: empty columns go, then the list
      // with one column left.
      final end = tester.getBottomLeft(editableWith('Heading 1')) + const Offset(40, -4);
      await dragBlock(tester, 'sc03', end);
      await dragBlock(tester, 'sc04', tester.getBottomLeft(editableWith('Heading 1')) + const Offset(40, -4));
      blocks = await blocksOf(tester, s.showcaseId);
      expect(blocks.where((b) => b.kind == 'columnlist' || b.kind == 'column'), isEmpty);
      expect(blocks.first.id, 'sc01');
      expect(blocks.first.depth, 0);
    });
  });

  group('link previews', () {
    test('reads title, description and icons from html', () {
      final p = parseLinkPreview('''
<html><head>
<title> Plain  title </title>
<meta property="og:title" content="Ferry &amp; bus times">
<meta name="description" content='Sailings &#8211; daily'>
<link rel="icon" href="/static/icon.svg" type="image/svg+xml">
<link rel="shortcut icon" href="favicon-32.png">
</head><body>...</body></html>''', Uri.parse('https://example.com/a/b'));
      expect(p.title, 'Ferry & bus times');
      expect(p.description, 'Sailings – daily');
      expect(p.icons.map((u) => u.toString()), ['https://example.com/a/favicon-32.png', 'https://example.com/favicon.ico']);
      final q = parseLinkPreview('<title>Only &lt;this&gt;</title>', Uri.parse('http://x.test/'));
      expect(q.title, 'Only <this>');
      expect(q.description, isNull);
    });

    test('fetches a page and its favicon from a server, and fails quietly', () async {
      // The test binding answers every request with 400; talk to a real
      // server on this machine instead.
      final overrides = HttpOverrides.current;
      HttpOverrides.global = null;
      addTearDown(() => HttpOverrides.global = overrides);
      final server = await HttpServer.bind(InternetAddress.loopbackIPv4, 0);
      addTearDown(() => server.close(force: true));
      server.listen((req) {
        final r = req.response;
        if (req.uri.path == '/page') {
          r.headers.contentType = ContentType.html;
          r.write('<head><title>Local page</title><meta name="description" content="Served here."></head>');
        } else if (req.uri.path == '/favicon.ico') {
          r.headers.contentType = ContentType('image', 'x-icon');
          r.add([1, 2, 3, 4]);
        } else {
          r.statusCode = 404;
        }
        r.close();
      });
      final base = 'http://127.0.0.1:${server.port}';
      final p = await fetchLinkPreview('$base/page');
      expect(p!.title, 'Local page');
      expect(p.description, 'Served here.');
      expect(p.icon, [1, 2, 3, 4]);
      expect(await fetchLinkPreview('$base/missing'), isNull);
      expect(await fetchLinkPreview('http://127.0.0.1:1/nothing'), isNull);
      expect(await fetchLinkPreview('not a url'), isNull);
    });

    test('the cache file keeps previews across starts', () async {
      final dir = tempDir('notes_links_');
      final f = File(join(dir.path, 'link_previews.json'));
      var fetches = 0;
      final a = LinkPreviews(file: f, fetch: (u) async {
        fetches++;
        return LinkPreview(title: 'T $u', icon: Uint8List.fromList([9, 8]));
      });
      expect(a.of('https://a.test/'), isNull);
      await Future<void>.delayed(Duration.zero);
      expect(a.of('https://a.test/')!.title, 'T https://a.test/');
      a.dispose(); // writes the pending save
      final b = LinkPreviews(file: f, fetch: (u) async {
        fetches++;
        return null;
      });
      expect(b.of('https://a.test/')!.icon, [9, 8]);
      expect(fetches, 1);
      b.dispose();
    });
  });
}
