import 'dart:ui' as ui;

import 'package:flutter/gestures.dart';
import 'package:flutter/widgets.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:notes_app/editor/rich_controller.dart';
import 'package:notes_app/addons/tasks/api.dart' show ShortItemCard;
import 'package:notes_app/core/models.dart';
import 'package:notes_app/state/agenda_store.dart';
import 'package:notes_app/ui/confetti.dart';
import 'package:notes_app/ui/space_wiggle.dart';
import 'package:notes_app/ui/ui.dart';

import 'harness.dart';
import 'stacks_calico_fixture.dart';

Widget themed(Widget child, {bool reduced = false}) => MediaQuery(
  data: MediaQueryData(disableAnimations: reduced),
  child: AppTheme(
    colors: AppColors.light,
    formFactor: FormFactor.desktop,
    child: Directionality(
      textDirection: TextDirection.ltr,
      child: Center(child: child),
    ),
  ),
);

void main() {
  setUpAll(initCore);

  testWidgets('mention paints a three-pixel target patch below the baseline', (
    tester,
  ) async {
    final controller = RichController(
      text: 'Visit Trips.',
      marks: [
        const BMark(start: 6, end: 11, kind: BMarkKind.pageLink, url: 'trips'),
      ],
    );
    final fill = spacePatch('sky', Brightness.light)!.fill;
    controller.pageLinkFill = (_) => fill;
    final editable = GlobalKey<EditableTextState>();
    final focus = FocusNode();
    await tester.pumpWidget(
      themed(
        SizedBox(
          width: 300,
          height: 60,
          child: EditableText(
            key: editable,
            controller: controller,
            focusNode: focus,
            style: const TextStyle(fontSize: 20, height: 1.5),
            cursorColor: const Color(0xFF000000),
            backgroundCursorColor: const Color(0xFF000000),
          ),
        ),
      ),
    );
    final render = editable.currentState!.renderEditable;
    final box = render
        .getBoxesForSelection(
          const TextSelection(baseOffset: 6, extentOffset: 11),
        )
        .single
        .toRect();
    final metrics = TextPainter(
      text: render.text,
      textDirection: TextDirection.ltr,
    )..layout(maxWidth: 300);
    final baseline = metrics.computeLineMetrics().single.baseline;
    metrics.dispose();
    Future<List<int>> alphaColumn() async {
      final recorder = ui.PictureRecorder();
      MarkPainter(
        controller: controller,
        editable: editable,
        colors: AppColors.light,
        textScaler: TextScaler.noScaling,
      ).paint(Canvas(recorder), const Size(300, 60));
      final picture = recorder.endRecording();
      final raster = (await tester.runAsync(() => picture.toImage(300, 60)))!;
      final data = (await tester.runAsync(
        () => raster.toByteData(format: ui.ImageByteFormat.rawRgba),
      ))!;
      final x = box.center.dx.floor();
      final result = [
        for (var y = 0; y < 60; y++) data.getUint8((y * 300 + x) * 4 + 3),
      ];
      raster.dispose();
      picture.dispose();
      return result;
    }

    final alphas = await alphaColumn();
    final rows = [
      for (var y = 0; y < alphas.length; y++)
        if (alphas[y] > 0) y,
    ];
    expect(rows.first, (baseline + 2).floor());
    expect(alphas.reduce((a, b) => a + b), closeTo(3 * 255, 4));
    controller.pageLinkFill = (_) => null;
    expect((await alphaColumn()).every((a) => a == 0), isTrue);
    await tester.pumpWidget(const SizedBox.shrink());
    controller.dispose();
    focus.dispose();
  });

  testWidgets(
    'space markers are eight-pixel rounded patches and none is absent',
    (tester) async {
      await tester.pumpWidget(
        themed(
          const Row(
            mainAxisSize: MainAxisSize.min,
            children: [SpaceDot('moss'), SpaceDot('none'), SpaceDot(null)],
          ),
        ),
      );
      final markers = find.descendant(
        of: find.byType(SpaceDot),
        matching: find.byType(Container),
      );
      expect(markers, findsOneWidget);
      expect(tester.getSize(markers), const Size(8, 8));
      final d = tester.widget<Container>(markers).decoration! as BoxDecoration;
      expect(d.color, spacePatch('moss', Brightness.light)!.fill);
      expect(d.borderRadius, BorderRadius.circular(2));
    },
  );

  for (final reduced in [false, true]) {
    testWidgets('space tile wiggles once on hover, reduced=$reduced', (
      tester,
    ) async {
      await tester.pumpWidget(
        themed(
          const SpaceWiggle(child: SizedBox(width: 22, height: 22)),
          reduced: reduced,
        ),
      );
      final mouse = await tester.createGesture(kind: PointerDeviceKind.mouse);
      await mouse.addPointer(location: Offset.zero);
      await mouse.moveTo(tester.getCenter(find.byType(SpaceWiggle)));
      await tester.pump();
      await tester.pump(const Duration(milliseconds: 50));
      final transforms = find.descendant(
        of: find.byType(SpaceWiggle),
        matching: find.byType(Transform),
      );
      if (reduced) {
        expect(transforms, findsNothing);
        expect(tester.binding.hasScheduledFrame, isFalse);
      } else {
        expect(
          tester.widget<Transform>(transforms).transform.storage[1],
          closeTo(.0523, .002),
        );
        await tester.pump(const Duration(milliseconds: 200));
        expect(
          tester.widget<Transform>(transforms).transform.storage[1],
          closeTo(0, .00001),
        );
        expect(tester.binding.hasScheduledFrame, isFalse);
      }
      await mouse.removePointer();
    });

    testWidgets(
      'confetti has 14 pieces, lasts 900ms and passes taps, reduced=$reduced',
      (tester) async {
        var revision = 0, taps = 0;
        late StateSetter update;
        await tester.pumpWidget(
          themed(
            StatefulBuilder(
              builder: (context, setState) {
                update = setState;
                return ConfettiBurst(
                  trigger: revision,
                  seed: 7,
                  patch: spacePatch('moss', Brightness.light)!,
                  child: GestureDetector(
                    behavior: HitTestBehavior.opaque,
                    onTap: () => taps++,
                    child: const SizedBox(width: 220, height: 30),
                  ),
                );
              },
            ),
            reduced: reduced,
          ),
        );
        final state = tester.state<ConfettiBurstState>(
          find.byType(ConfettiBurst),
        );
        expect(state.bursts, 0);
        expect(find.byKey(const ValueKey('confetti-pieces')), findsNothing);
        update(() => revision++);
        await tester.pump();
        await tester.pump(const Duration(milliseconds: 250));
        expect(state.bursts, reduced ? 0 : 1);
        if (!reduced) {
          expect(state.pieces, hasLength(14));
          for (final piece in state.pieces) {
            expect(piece.width, inInclusiveRange(6, 9));
            expect(piece.height, inInclusiveRange(6, 9));
          }
          expect(find.byKey(const ValueKey('confetti-pieces')), findsOneWidget);
        }
        await tester.tap(find.byType(ConfettiBurst));
        expect(taps, 1);
        update(() {});
        await tester.pump();
        expect(state.bursts, reduced ? 0 : 1);
        await tester.pump(const Duration(milliseconds: 651));
        expect(find.byKey(const ValueKey('confetti-pieces')), findsNothing);
      },
    );
  }

  for (final dark in [false, true]) {
    for (final mobile in [false, true]) {
      for (final color in ['moss', 'none']) {
        appTest(
          'page space band, icon and tick follow root colour $color dark=$dark mobile=$mobile',
          (tester) async {
            await pumpApp(tester, mobile: mobile, dark: dark, empty: true);
            final f = await seedCalico(tester, color: color);
            await openCalico(tester, f.page, 'Weekend reading');
            final band = find.byKey(ValueKey('page-space-band-${f.page}'));
            final brightness = dark ? Brightness.dark : Brightness.light;
            final colors = dark ? AppColors.darkTheme : AppColors.light;
            final patch = spacePatch(color, brightness);
            expect(band, patch == null ? findsNothing : findsOneWidget);
            if (patch != null) {
              expect(tester.getSize(band).height, 6);
              expect(
                tester
                    .widget<ColoredBox>(
                      find.descendant(
                        of: band,
                        matching: find.byType(ColoredBox),
                      ),
                    )
                    .color,
                patch.fill,
              );
            }
            final box = tester.widget<StacksCheckbox>(
              find.byKey(const ValueKey('check-calico-one')),
            );
            expect(box.fill ?? colors.accent, patch?.fill ?? colors.accent);
            expect(box.ink ?? colors.onAccent, patch?.ink ?? colors.onAccent);
            final titleTile = tester.widget<Container>(
              find.descendant(
                of: find.byKey(const ValueKey('page-icon')),
                matching: find.byWidgetPredicate(
                  (w) => w is Container && w.constraints?.maxWidth == 64,
                ),
              ),
            );
            expect(
              (titleTile.decoration! as BoxDecoration).color,
              patch?.fill ?? colors.bgPrimary,
            );
            final tileFinder = find.byKey(
              ValueKey('rail-space-tile-${f.space}'),
            );
            expect(
              tileFinder,
              patch == null || mobile ? findsNothing : findsOneWidget,
            );
            if (patch != null && !mobile) {
              expect(
                tester.widget<SpaceIconTile>(tileFinder).patch!.fill,
                patch.fill,
              );
            }
            final mention =
                tester
                        .widget<EditableText>(
                          editableWith('Bring your book on Trips.'),
                        )
                        .controller
                    as RichController;
            expect(
              mention.pageLinkFill?.call(f.target),
              spacePatch('sky', brightness)!.fill,
            );
            await core(
              tester,
              (v) => v.run(
                (r) => r.setPageColor(page: f.target, color: 'none'),
                writes: true,
              ),
            );
            await settle(tester);
            expect(mention.pageLinkFill?.call(f.target), isNull);
            if (!mobile) {
              await openCalico(tester, f.target, 'Trips', from: f.page);
              final spine = tester.widget<Container>(
                find
                    .descendant(
                      of: find.byKey(ValueKey('stack-spine-${f.page}')),
                      matching: find.byType(Container),
                    )
                    .first,
              );
              expect(
                (spine.decoration! as BoxDecoration).color,
                patch?.fill ?? colors.bgPrimary,
              );
              // Home has no spine: the pile shows only the sheet below.
              expect(
                find.byKey(const ValueKey('stack-spine-home')),
                findsNothing,
              );
            }
          },
        );
      }
    }
  }

  appTest('rail duvet tile stays visible in the light theme', (tester) async {
    await pumpApp(tester, mobile: false, empty: true);
    final f = await seedCalico(tester, color: 'duvet');
    final tile = tester.widget<SpaceIconTile>(
      find.byKey(ValueKey('rail-space-tile-${f.space}')),
    );
    expect(tile.size, 22);
    expect(tile.patch!.fill, const Color(0xFFDCD4C8));
    expect(tile.patch!.ink, const Color(0xFF282A2F));
  });

  appTest('cat leaves an empty page after first-line typing', (tester) async {
    await pumpApp(tester, mobile: false, empty: true);
    final f = await seedCalico(tester);
    await openCalico(tester, f.empty, 'A quiet page');
    expect(find.byKey(const ValueKey('empty-page-cat')), findsOneWidget);
    expect(
      find.text('Nothing here yet. Start typing, or press / for blocks.'),
      findsOneWidget,
    );
    final empty = find
        .byWidgetPredicate(
          (w) =>
              w is EditableText &&
              w.key is GlobalKey &&
              w.controller.text.isEmpty,
        )
        .first;
    await focusAt(tester, empty, 0);
    await typeText(tester, 'A new thought');
    expect(find.byKey(const ValueKey('empty-page-cat')), findsNothing);
    expect(editableWith('A new thought'), findsOneWidget);
  });

  appTest(
    'editor completion celebrates a user tick once and not undo or load',
    (tester) async {
      await pumpApp(tester, mobile: false, empty: true);
      final f = await seedCalico(tester);
      await openCalico(tester, f.page, 'Weekend reading');
      final burst = find.byKey(
        const ValueKey('heading-confetti-calico-heading'),
      );
      final state = tester.state<ConfettiBurstState>(burst);
      expect(state.bursts, 0);
      await tester.tap(find.byKey(const ValueKey('check-calico-two')));
      await tester.pump();
      expect(find.text('All done'), findsOneWidget);
      expect(state.bursts, 1);
      await tester.pump(const Duration(milliseconds: 900));
      calicoEditor(tester, f.page).undo();
      await settle(tester);
      expect(find.text('1 of 2'), findsOneWidget);
      expect(state.bursts, 1);
    },
  );

  appTest('completed checklist is quiet on load', (tester) async {
    await pumpApp(tester, mobile: false, empty: true);
    final f = await seedCalico(tester, complete: true);
    await openCalico(tester, f.page, 'Weekend reading');
    final state = tester.state<ConfettiBurstState>(
      find.byKey(const ValueKey('heading-confetti-calico-heading')),
    );
    expect(find.text('All done'), findsOneWidget);
    expect(state.bursts, 0);
    expect(find.byKey(const ValueKey('confetti-pieces')), findsNothing);
  });

  for (final reduced in [false, true]) {
    appTest(
      'Today last due tick persists short-card completion, reduced=$reduced',
      (tester) async {
        final tv = await pumpApp(tester, mobile: false, empty: true);
        tester.platformDispatcher.accessibilityFeaturesTestValue =
            FakeAccessibilityFeatures(disableAnimations: reduced);
        await core(tester, (vault) async {
          final page = await vault.createPage(title: 'Garden');
          await vault.run(
            (v) => v.setPageColor(page: page, color: 'moss'),
            writes: true,
          );
          await vault.apply(page, [
            const BOp(
              op: OpKind.insert,
              id: 'today-calico-task',
              kind: Kinds.todo,
              text: 'Water seedlings @today',
              checked: false,
              marks: [
                BMark(
                  start: 16,
                  end: 22,
                  kind: BMarkKind.date,
                  url: '2026-10-06',
                ),
              ],
            ),
          ]);
          await vault.flush();
        });
        await finishAsync(tester, tv.app.agenda.reload());
        await settle(tester);
        final item = tv.app.agenda.items.singleWhere(
          (i) => i.block == 'today-calico-task',
        );
        final row = find.byKey(
          ValueKey('today-item-${AgendaStore.itemKey(item)}'),
        );
        final check = find.descendant(
          of: row,
          matching: find.byType(StacksCheckbox),
        );
        await tester.ensureVisible(check);
        expect(tester.widget<StacksCheckbox>(check).checked, isFalse);
        expect(tester.widget<ShortItemCard>(row).item.spaceColor, 'moss');
        await tapAndSettle(tester, check);
        await settleUntil(
          tester,
          () =>
              tv.app.agenda.selectedDoneItems.any((i) => i.block == item.block),
        );
        expect(tester.widget<StacksCheckbox>(check).checked, isTrue);
        expect(find.byKey(const ValueKey('today-done')), findsOneWidget);
        final page = await pageOf(tester, item.page);
        expect(
          page.blocks.singleWhere((b) => b.id == item.block).checked,
          isTrue,
        );
        await finishAsync(tester, tv.app.agenda.reload());
        await settle(tester);
        expect(tester.widget<StacksCheckbox>(check).checked, isTrue);
        expect(tester.widget<ShortItemCard>(row).item.spaceColor, 'moss');
      },
    );
  }

  appTest(
    'editor completion keeps All done without confetti under reduced motion',
    (tester) async {
      await pumpApp(tester, mobile: false, empty: true);
      tester.platformDispatcher.accessibilityFeaturesTestValue =
          const FakeAccessibilityFeatures(disableAnimations: true);
      final f = await seedCalico(tester);
      await openCalico(tester, f.page, 'Weekend reading');
      await tester.tap(find.byKey(const ValueKey('check-calico-two')));
      await tester.pump();
      expect(find.text('All done'), findsOneWidget);
      expect(
        tester
            .state<ConfettiBurstState>(
              find.byKey(const ValueKey('heading-confetti-calico-heading')),
            )
            .bursts,
        0,
      );
      expect(find.byKey(const ValueKey('confetti-pieces')), findsNothing);
    },
  );
}
