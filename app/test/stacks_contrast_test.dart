import 'package:flutter/widgets.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:notes_app/ui/tokens.dart';
import 'package:notes_app/ui/block_colors.dart';
import 'package:notes_app/ui/database/db_theme.dart';

// Every Stacks text colour meets body-text AA (4.5:1) on sheet, desk and
// inset, and rail text on the rail, in both themes.
double contrastRatio(Color foreground, Color background) {
  final a = Color.alphaBlend(foreground, background).computeLuminance();
  final b = background.computeLuminance();
  return a > b ? (a + .05) / (b + .05) : (b + .05) / (a + .05);
}

void main() {
  test('Calico patch ink and spine hover retain body-text AA', () {
    for (final brightness in Brightness.values) {
      for (final name in spaceColorNames) {
        final patch = spacePatch(name, brightness)!;
        expect(
          contrastRatio(patch.ink, patch.fill),
          greaterThanOrEqualTo(4.5),
          reason: '$brightness $name ink/fill',
        );
        final hover = Color.alphaBlend(
          patch.ink.withValues(alpha: .12),
          patch.fill,
        );
        expect(patch.hover, hover);
        // Moss tints the strip, retaining its original backing under the title.
        final titleBacking = name == 'moss' ? patch.fill : hover;
        expect(
          contrastRatio(patch.ink, titleBacking),
          greaterThanOrEqualTo(4.5),
          reason: '$brightness $name spine hover title',
        );
        expect(patch.hoverTextBacking, titleBacking);
        final rail = spacePatch(name, brightness, rail: true)!;
        expect(
          contrastRatio(rail.ink, rail.fill),
          greaterThanOrEqualTo(4.5),
          reason: '$brightness $name rail tile',
        );
      }
    }
    expect(spacePatch(null, Brightness.light), isNull);
    expect(spacePatch('none', Brightness.dark), isNull);
    expect(
      spacePatch('duvet', Brightness.light, rail: true)!.fill,
      const Color(0xFFDCD4C8),
    );
  });
  test('Stacks text contrast across sheets, desks, insets and rails', () {
    final bodyFailures = <String>[];
    for (final theme in {
      'light': AppColors.light,
      'dark': AppColors.darkTheme,
    }.entries) {
      final c = theme.value;
      final text = {
        'textPrimary': c.textPrimary,
        'textSecondary': c.textSecondary,
        'textTertiary': c.textTertiary,
        'accentText': c.accentText,
      };
      final surfaces = {
        'sheet': c.bgPrimary,
        'desk': c.desk,
        'inset': c.bgSecondary,
      };
      for (final fg in text.entries) {
        for (final bg in surfaces.entries) {
          final label = '${theme.key}.${fg.key}/${bg.key}';
          final ratio = contrastRatio(fg.value, bg.value);
          expect(
            ratio,
            greaterThanOrEqualTo(3),
            reason: '$label: large-text AA',
          );
          if (ratio < 4.5) bodyFailures.add(label);
        }
      }
      for (final fg in {
        'railInk': c.railInk,
        'railMuted': c.railMuted,
      }.entries) {
        for (final bg in {'rail': c.rail, 'rail2': c.rail2}.entries) {
          expect(
            contrastRatio(fg.value, bg.value),
            greaterThanOrEqualTo(4.5),
            reason: '${theme.key}.${fg.key}/${bg.key}: body-text AA',
          );
        }
      }
    }
    expect(bodyFailures, isEmpty);
  });
  test('Stacks status, callout and event-pill text keeps body-text AA', () {
    for (final c in [AppColors.light, AppColors.darkTheme]) {
      expect(
        contrastRatio(c.bgPrimary, c.textPrimary),
        greaterThanOrEqualTo(4.5),
        reason: 'today date number',
      );
      expect(
        contrastRatio(c.onAccent, c.accent),
        greaterThanOrEqualTo(4.5),
        reason: 'accent action',
      );
      for (final base in [c.bgPrimary, c.desk, c.bgSecondary]) {
        for (final fill in [c.okBg, c.infoBg, c.attnBg, c.dangerBg, c.pawBg]) {
          expect(
            contrastRatio(c.textPrimary, Color.alphaBlend(fill, base)),
            greaterThanOrEqualTo(4.5),
          );
        }
        for (final name in tagColors) {
          final tag = c.tag(name);
          expect(
            contrastRatio(tag.text, Color.alphaBlend(tag.bg, base)),
            greaterThanOrEqualTo(4.5),
            reason: '${c.dark ? 'dark' : 'light'} $name option',
          );
        }
        for (final name in spaceColorNames) {
          final color = spaceColor(
            name,
            c.dark ? Brightness.dark : Brightness.light,
          )!;
          expect(
            contrastRatio(
              c.textPrimary,
              Color.alphaBlend(color.withValues(alpha: .25), base),
            ),
            greaterThanOrEqualTo(4.5),
            reason: '$name event pill',
          );
        }
      }
      for (final name in paletteNames) {
        expect(
          contrastRatio(
            c.textPrimary,
            Color.alphaBlend(paletteBackground(name, c)!, c.bgPrimary),
          ),
          greaterThanOrEqualTo(4.5),
          reason: '$name editable placeholder',
        );
        final color = paletteText(name, c)!;
        for (final surface in [
          c.bgPrimary,
          c.bgSecondary,
          c.desk,
          Color.alphaBlend(c.attnBg, c.bgPrimary),
          Color.alphaBlend(c.bgSelectedBlock, c.bgPrimary),
          for (final fillName in paletteNames) calloutFill(fillName, c),
          for (final fillName in paletteNames)
            Color.alphaBlend(paletteBackground(fillName, c)!, c.bgPrimary),
        ]) {
          expect(
            contrastRatio(color, surface),
            greaterThanOrEqualTo(4.5),
            reason: '${c.dark ? 'dark' : 'light'} $name text',
          );
        }
      }
    }
  });
}
