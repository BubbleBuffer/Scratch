// A page with one of every Phase 2 block (toggle, callout, columns, media,
// links, mentions, colours), an icon and a cover, for the pages tests and
// goldens.

import 'package:flutter/widgets.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:notes_app/core/models.dart';
import 'package:notes_app/blocks/attachment.dart';
import 'package:notes_app/features/page/cover_picker.dart';
import 'package:notes_app/shell/home.dart';
import 'package:notes_app/shell/navigation.dart';

import 'harness.dart';

const richTitle = 'Trip planning';

String? _richId;

/// Ids of the rich page and its blocks.
class RichPage {
  RichPage(this.id, this.image, this.cover);
  final String id;
  final String image;
  final String cover;
}

/// A small PNG (a two-colour gradient), rendered without the engine's
/// image codecs so it works under `runAsync`.
Future<List<int>> testPng(List<Color> colors, {int w = 600, int h = 300}) => renderCoverPng(colors, w: w, h: h);

BOp _ins(String id, String kind, String text,
        {String? parent,
        String? after,
        String? target,
        String? color,
        int? width,
        bool checked = false,
        List<BMark> marks = const []}) =>
    BOp(
      op: OpKind.insert,
      id: id,
      parent: parent,
      after: after,
      kind: kind,
      text: text,
      marks: marks,
      checked: checked,
      target: target,
      color: color,
      width: width,
    );

BMark _m(String text, String sub, BMarkKind kind, [String url = '']) {
  final i = text.indexOf(sub);
  return BMark(start: i, end: i + sub.length, kind: kind, url: url);
}

Future<RichPage> createRichPage(WidgetTester tester, TestVault tv) async {
  // Attachment ids are content hashes: forget paths from earlier tests' vaults.
  clearAttachmentCache();
  final r = await tester.runAsync(() async {
    final v = tv.vault;
    final id = await createFixtureSpace(v, title: richTitle);
    final png = await testPng(const [Color(0xFF7AA6C2), Color(0xFFE3C59B)]);
    final image = await v.run((x) => x.addAttachmentBytes(name: 'harbour.png', bytes: png), writes: true);
    final coverPng = await testPng(const [Color(0xFFF2D5C4), Color(0xFFB9D3E8)], w: 1500, h: 600);
    final cover = await v.run((x) => x.addAttachmentBytes(name: 'cover.png', bytes: coverPng), writes: true);
    final file = await v.run((x) => x.addAttachmentBytes(name: 'itinerary.pdf', bytes: List.filled(48213, 7)), writes: true);
    await v.setIcon(id, '🧭');
    await v.run((x) => x.setCover(id: id, cover: cover), writes: true);
    final recipes = tv.pages['Recipes']!;
    const p1 = 'Book the ferry with Recipes in mind before @Friday, and pack light.';
    const p2 = 'Colours: red text, a yellow highlight and underlined words.';
    final ops = <BOp>[
      _ins('r01', Kinds.toc, ''),
      _ins('r02', Kinds.heading2, 'Plan', after: 'r01'),
      _ins('r03', Kinds.paragraph, p1, after: 'r02', marks: [
        _m(p1, 'Recipes', BMarkKind.pageLink, recipes),
        _m(p1, '@Friday', BMarkKind.date, '2026-10-02'),
      ]),
      _ins('r04', Kinds.paragraph, p2, after: 'r03', marks: [
        _m(p2, 'red text', BMarkKind.color, 'red'),
        _m(p2, 'a yellow highlight', BMarkKind.color, 'yellow_background'),
        _m(p2, 'underlined words', BMarkKind.underline),
      ]),
      _ins('r05', Kinds.callout, 'Ferries leave every two hours; the last one is at 21:10.', after: 'r04', target: '⛴️'),
      _ins('r06', Kinds.toggle, 'Packing list', after: 'r05'),
      _ins('r07', Kinds.todo, 'Passport', parent: 'r06', checked: true),
      _ins('r08', Kinds.todo, 'Rain jacket', parent: 'r06', after: 'r07'),
      _ins('r09', Kinds.toggleHeading3, 'Budget notes', after: 'r06'),
      _ins('r10', Kinds.paragraph, 'Hidden until opened.', parent: 'r09'),
      _ins('r11', Kinds.heading2, 'Places', after: 'r09'),
      _ins('r12', Kinds.columnList, '', after: 'r11'),
      _ins('r13', Kinds.column, '', parent: 'r12'),
      _ins('r14', Kinds.heading3, 'Morning', parent: 'r13'),
      _ins('r15', Kinds.bulleted, 'Harbour walk', parent: 'r13', after: 'r14'),
      _ins('r16', Kinds.column, '', parent: 'r12', after: 'r13'),
      _ins('r17', Kinds.heading3, 'Evening', parent: 'r16'),
      _ins('r18', Kinds.bulleted, 'Fish market', parent: 'r16', after: 'r17'),
      _ins('r19', Kinds.image, 'The harbour at noon', after: 'r12', target: image, width: 70),
      _ins('r20', Kinds.file, '', after: 'r19', target: file),
      _ins('r21', Kinds.bookmark, '', after: 'r20', target: 'https://www.example.com/ferries/timetable'),
      _ins('r22', Kinds.linkToPage, '', after: 'r21', target: recipes),
      _ins('r23', Kinds.paragraph, 'Blue background block.', after: 'r22', color: 'blue_background'),
      _ins('r24', Kinds.code,
          'const timetable = ["07:10", "09:10", "11:10", "13:10", "15:10", "17:10", "19:10", "21:10"]; // every two hours',
          after: 'r23'),
    ];
    // Insert carries no target, colour or width: those are their own ops.
    final all = <BOp>[
      for (final o in ops) ...[
        BOp(op: o.op, id: o.id, parent: o.parent, after: o.after, kind: o.kind, text: o.text, marks: o.marks, checked: o.checked),
        if (o.target != null)
          BOp(op: OpKind.setTarget, id: o.id, kind: o.kind, text: '', marks: const [], checked: false, target: o.target),
        if (o.color != null)
          BOp(op: OpKind.setColor, id: o.id, kind: o.kind, text: '', marks: const [], checked: false, color: o.color),
        if (o.width != null)
          BOp(op: OpKind.setWidth, id: o.id, kind: o.kind, text: '', marks: const [], checked: false, width: o.width),
      ],
    ];
    await v.apply(id, all);
    await v.flush();
    _richId = id;
    return RichPage(id, image, cover);
  });
  await settle(tester);
  return r!;
}

/// Opens page [title] of the test vault (through the app's navigation, as
/// the sidebar or home list would).
Future<void> openByTitle(WidgetTester tester, String title) async {
  final id = title == richTitle ? _richId! : current.pages[title]!;
  openPage(tester.element(find.byType(HomeScreen)), id, title);
  await settle(tester);
  // Let file images decode on the real clock.
  await settleImages(tester);
}

/// Waits (on the real clock) until every file image on screen is decoded.
Future<void> settleImages(WidgetTester tester) async {
  for (var round = 0; round < 3; round++) {
    await tester.runAsync(() => Future<void>.delayed(Duration.zero));
    await tester.pump(const Duration(milliseconds: 50));
    for (final e in find.byType(Image).evaluate()) {
      final img = e.widget as Image;
      await finishAsync(tester, precacheImage(img.image, e));
    }
    await tester.pump(const Duration(milliseconds: 50));
  }
  await settle(tester);
}
