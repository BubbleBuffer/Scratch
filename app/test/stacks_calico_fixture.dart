import 'dart:async';

import 'package:flutter/widgets.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:notes_app/core/models.dart';
import 'package:notes_app/editor/page_screen.dart';
import 'package:notes_app/shell/home.dart';
import 'package:notes_app/shell/navigation.dart';

import 'harness.dart';

class CalicoFixture {
  const CalicoFixture(this.space, this.page, this.target, this.empty);
  final String space, page, target, empty;
}

Future<CalicoFixture> seedCalico(
  WidgetTester tester, {
  String color = 'moss',
  bool complete = false,
}) async {
  final fixture = await core(tester, (vault) async {
    final space = await vault.createPage(title: 'Reading');
    await vault.run(
      (v) => v.setPageColor(page: space, color: color),
      writes: true,
    );
    final page = await vault.createPage(
      parent: space,
      title: 'Weekend reading',
    );
    await vault.setIcon(page, '📖');
    // A child colour must not override the root's space colour.
    await vault.run(
      (v) => v.setPageColor(page: page, color: 'paw'),
      writes: true,
    );
    final target = await vault.createPage(title: 'Trips');
    await vault.run(
      (v) => v.setPageColor(page: target, color: 'sky'),
      writes: true,
    );
    await vault.setIcon(target, '✈️');
    final empty = await vault.createPage(parent: space, title: 'A quiet page');
    await vault.apply(page, [
      const BOp(
        op: OpKind.insert,
        id: 'calico-heading',
        kind: Kinds.heading2,
        text: 'Reading checklist',
        marks: [],
        checked: false,
      ),
      const BOp(
        op: OpKind.insert,
        id: 'calico-one',
        after: 'calico-heading',
        kind: Kinds.todo,
        text: 'Choose a book',
        marks: [],
        checked: true,
      ),
      BOp(
        op: OpKind.insert,
        id: 'calico-two',
        after: 'calico-one',
        kind: Kinds.todo,
        text: 'Make a little time',
        marks: const [],
        checked: complete,
      ),
      BOp(
        op: OpKind.insert,
        id: 'calico-mention',
        after: 'calico-two',
        kind: Kinds.paragraph,
        text: 'Bring your book on Trips.',
        checked: false,
        marks: [
          BMark(start: 19, end: 24, kind: BMarkKind.pageLink, url: target),
        ],
      ),
    ]);
    await vault.flush();
    return CalicoFixture(space, page, target, empty);
  });
  await settle(tester);
  return fixture;
}

PageEditorState calicoEditor(WidgetTester tester, String id) =>
    tester.state<PageEditorState>(
      find.byWidgetPredicate((w) => w is PageScreen && w.pageId == id),
    );

Future<void> openCalico(
  WidgetTester tester,
  String id,
  String title, {
  String? from,
}) async {
  final context = from == null
      ? tester.element(find.byType(HomeScreen))
      : calicoEditor(tester, from).context;
  unawaited(openPage(context, id, title, stack: true));
  await settle(tester);
}

Future<void> calicoShot(String name) => expectLater(
  find.byType(Directionality).first,
  matchesGoldenFile('../screenshots/stacks-calico-$name.png'),
);
