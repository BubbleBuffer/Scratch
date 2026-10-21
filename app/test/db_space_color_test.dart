import 'dart:io';

import 'package:flutter/widgets.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:notes_app/core/vault.dart';
import 'package:notes_app/ui/database/db_theme.dart';
import 'package:notes_app/state/collection_store.dart';
import 'package:notes_app/ui/ui.dart';

import 'db_harness.dart';

void main() {
  setUpAll(initCore);
  test(
    'new top-level page stays uncoloured through boot backfill and reopen',
    () async {
      final dir = await Directory.systemTemp.createTemp(
        'notes_space_color_test_',
      );
      final root = '${dir.path}${Platform.pathSeparator}vault';
      var vault = await Vault.open(root);
      try {
        final existing = await vault.createPage(title: 'Existing space');
        await vault.run(
          (v) => v.setPageColor(page: existing, color: 'moss'),
          writes: true,
        );
        final page = await vault.createPage(title: 'New page');
        expect((await vault.loadPage(page)).color, 'none');
        expect(await vault.run((v) => v.ensureSpaceColors(), writes: true), 0);
        expect((await vault.loadPage(page)).color, 'none');
        expect((await vault.loadPage(existing)).color, 'moss');
        await vault.flush();
        vault.dispose();
        vault = await Vault.open(root);
        expect(await vault.run((v) => v.ensureSpaceColors(), writes: true), 0);
        expect((await vault.loadPage(page)).color, 'none');
        expect((await vault.loadPage(existing)).color, 'moss');
        expect(
          spaceColor((await vault.loadPage(page)).color, Brightness.light),
          isNull,
        );
        expect(eventPillFill(AppColors.light, 'none'), AppColors.light.infoBg);
        expect(
          eventPillFill(AppColors.darkTheme, 'none'),
          AppColors.darkTheme.infoBg,
        );
      } finally {
        vault.dispose();
        await dir.delete(recursive: true);
      }
    },
  );

  dbTest('database event colours follow the root space and its explicit none', (
    tester,
  ) async {
    await pumpDb(tester, mobile: false);
    final root = await core(tester, (vault) async {
      final root = await vault.createPage(title: 'Space');
      await vault.run(
        (v) => v.setPageColor(page: root, color: 'moss'),
        writes: true,
      );
      final intermediate = await vault.createPage(
        title: 'Nested page',
        parent: root,
      );
      await vault.run(
        (v) => v.setPageColor(page: intermediate, color: 'paw'),
        writes: true,
      );
      // Both the intermediate page and this collection have their own colours;
      // the existing space contract still resolves the top-level ancestor.
      await vault.movePage(db.projects, newParent: intermediate);
      return root;
    });
    final store = CollectionStore(
      db.vault,
      db.app.feed,
      db.projects,
      withRows: false,
    );
    addTearDown(store.dispose);
    await tester.runAsync(store.load);
    expect(store.resolvedSpaceColor, 'moss');
    await core(
      tester,
      (vault) => vault.run(
        (v) => v.setPageColor(page: root, color: 'sky'),
        writes: true,
      ),
    );
    await settleUntil(tester, () => store.resolvedSpaceColor == 'sky');
    await core(
      tester,
      (vault) => vault.run(
        (v) => v.setPageColor(page: root, color: 'none'),
        writes: true,
      ),
    );
    await settleUntil(tester, () => store.resolvedSpaceColor == 'none');
    expect(
      eventPillFill(AppColors.light, store.resolvedSpaceColor),
      AppColors.light.infoBg,
    );
    expect(
      eventPillFill(AppColors.darkTheme, store.resolvedSpaceColor),
      AppColors.darkTheme.infoBg,
    );
    await core(
      tester,
      (vault) => vault.run(
        (v) => v.setPageColor(page: root, color: 'apricot'),
        writes: true,
      ),
    );
    await settleUntil(tester, () => store.resolvedSpaceColor == 'apricot');
  });
}
