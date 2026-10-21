// Approved Stacks tokens from wiki/product/design.md and the direction
// prototype, plus the app-wide type scale and bundled fonts.
// No Material anywhere: this is the only theme the widgets read.

import 'package:flutter/widgets.dart';

export '../core/clock.dart' show clockNow;

/// Bundled fonts used identically on every platform and in golden tests.
String sansFamily = 'Atkinson Hyperlegible Next';
String monoFamily = 'Fragment Mono';
String displayFamily = 'Bricolage Grotesque';
const monoFallback = ['monospace'];
Color _rgba(int r, int g, int b, double a) => Color.fromRGBO(r, g, b, a);

/// Quiet space colours shared by the tree, agenda and checklist progress.
const spaceColorNames = ['apricot', 'paw', 'moss', 'duvet', 'sun', 'sky'];
Color? spaceColor(String? name, Brightness brightness) => switch (name) {
  'apricot' => const Color(0xFFDE945F),
  'paw' => const Color(0xFFDDA39D),
  'moss' => const Color(0xFF697A6B),
  'duvet' =>
    brightness == Brightness.dark
        ? const Color(0xFF999DA6)
        : const Color(0xFF2F3138),
  'sun' => const Color(0xFFF0D29C),
  'sky' => const Color(0xFFCBD5DF),
  _ => null,
};

/// Opaque edge/tile colours; the quiet dot/pill palette above stays separate.
class SpacePatch {
  const SpacePatch(this.fill, this.deep, this.ink);
  final Color fill, deep, ink;

  Color get hover => Color.alphaBlend(ink.withValues(alpha: .12), fill);

  /// Moss keeps its original backing behind title/glyphs on hover.
  Color get hoverTextBacking => fill == const Color(0xFF5F7062) ? fill : hover;
}

SpacePatch? spacePatch(
  String? name,
  Brightness brightness, {
  bool rail = false,
}) {
  final dark = brightness == Brightness.dark || (rail && name == 'duvet');
  final values = switch (name) {
    'apricot' =>
      dark ? [0xDB8C56, 0xB0673A, 0x1E2024] : [0xDE945F, 0xBF6F3C, 0x282A2F],
    'paw' =>
      dark ? [0xD29891, 0xAC736D, 0x1E2024] : [0xDDA39D, 0xBC7C75, 0x282A2F],
    'moss' =>
      dark ? [0x5F7062, 0x4A5A4D, 0xFFFAF1] : [0x5F7062, 0x495A4C, 0xFFFAF1],
    'duvet' =>
      dark ? [0xDCD4C8, 0xB5AA9C, 0x282A2F] : [0x2F3138, 0x4C505A, 0xF3EFE7],
    'sun' =>
      dark ? [0xE3C48B, 0xBC9A58, 0x1E2024] : [0xF0D29C, 0xCDA663, 0x282A2F],
    'sky' =>
      dark ? [0xB9C6D3, 0x93A5B8, 0x1E2024] : [0xCBD5DF, 0xA3B3C4, 0x282A2F],
    _ => null,
  };
  return values == null
      ? null
      : SpacePatch(
          Color(0xFF000000 | values[0]),
          Color(0xFF000000 | values[1]),
          Color(0xFF000000 | values[2]),
        );
}

class AppColors {
  const AppColors({
    required this.dark,
    required this.bgPrimary,
    required this.bgSecondary,
    required this.bgTertiary,
    required this.bgElevated,
    required this.bgHover,
    required this.bgPressed,
    required this.bgSelectedBlock,
    required this.textSelection,
    required this.textPrimary,
    required this.textSecondary,
    required this.textTertiary,
    required this.textDisabled,
    required this.iconPrimary,
    required this.iconSecondary,
    required this.iconTertiary,
    required this.border,
    required this.borderSubtle,
    required this.borderStrong,
    required this.borderTranslucent,
    required this.divider,
    required this.accent,
    required this.accentHover,
    required this.inlineCodeBg,
    required this.inlineCodeText,
    required this.overlay,
    required this.desk,
    required this.accentText,
    required this.onAccent,
    required this.focus,
    required this.thread,
    required this.threadStrong,
    required this.okBg,
    required this.infoBg,
    required this.attnBg,
    required this.dangerBg,
    required this.pawBg,
    required this.rail,
    required this.rail2,
    required this.railInk,
    required this.railMuted,
    required this.railLine,
    required this.seam,
    required this.seamHover,
    required this.stack,
  });
  final bool dark;
  final Color bgPrimary;
  final Color bgSecondary;
  final Color bgTertiary;
  final Color bgElevated;
  final Color bgHover;
  final Color bgPressed;
  final Color bgSelectedBlock;
  final Color textSelection;
  final Color textPrimary;
  final Color textSecondary;
  final Color textTertiary;
  final Color textDisabled;
  final Color iconPrimary;
  final Color iconSecondary;
  final Color iconTertiary;
  final Color border;
  final Color borderSubtle;
  final Color borderStrong;
  final Color borderTranslucent;
  final Color divider;
  final Color accent;
  final Color accentHover;
  final Color inlineCodeBg;
  final Color inlineCodeText;
  final Color overlay;
  final Color desk;
  final Color accentText;
  final Color onAccent;
  final Color focus;
  final Color thread;
  final Color threadStrong;
  final Color okBg;
  final Color infoBg;
  final Color attnBg;
  final Color dangerBg;
  final Color pawBg;
  final Color rail;
  final Color rail2;
  final Color railInk;
  final Color railMuted;
  final Color railLine;
  final List<BoxShadow> seam, seamHover, stack;
  // Compatibility for screens that still refer to the old shadow names.
  List<BoxShadow> get shadowMenu => seamHover;
  List<BoxShadow> get shadowSmall => seam;

  static final light = AppColors(
    dark: false,
    bgPrimary: const Color(0xFFFFFAF1),
    bgSecondary: const Color(0xFFF3EFE7),
    bgTertiary: const Color(0xFFECE6DB),
    bgElevated: const Color(0xFFFFFAF1),
    bgHover: _rgba(40, 42, 47, 0.05),
    bgPressed: _rgba(40, 42, 47, 0.08),
    bgSelectedBlock: _rgba(222, 148, 95, 0.14),
    textSelection: _rgba(222, 148, 95, 0.30),
    textPrimary: const Color(0xFF282A2F),
    textSecondary: const Color(0xFF625C55),
    textTertiary: const Color(0xFF6C645D),
    textDisabled: const Color(0xFFB8ADA1),
    iconPrimary: const Color(0xFF282A2F),
    iconSecondary: const Color(0xFF625C55),
    iconTertiary: const Color(0xFF6C645D),
    border: const Color(0xFFE4DCCF),
    borderSubtle: const Color(0xFFEDE6DA),
    borderStrong: const Color(0xFFB8ADA1),
    borderTranslucent: const Color(0xFFE4DCCF),
    divider: const Color(0xFFE4DCCF),
    accent: const Color(0xFFDE945F),
    accentHover: const Color(0xFFC97F4A),
    inlineCodeBg: const Color(0xFFF3EFE7),
    inlineCodeText: const Color(0xFF874016),
    overlay: _rgba(40, 42, 47, 0.45),
    desk: const Color(0xFFECE6DB),
    accentText: const Color(0xFF874016),
    onAccent: const Color(0xFF282A2F),
    focus: const Color(0xFF7AB7FF),
    thread: const Color(0xFFB8ADA1),
    threadStrong: const Color(0xFF6C645D),
    okBg: const Color(0xFFCAD8CD),
    infoBg: const Color(0xFFCBD5DF),
    attnBg: const Color(0xFFF0D29C),
    dangerBg: const Color(0xFFE5B1AA),
    pawBg: const Color(0xFFEFD3CF),
    rail: const Color(0xFF2F3138),
    rail2: const Color(0xFF3B3E45),
    railInk: const Color(0xFFF3EFE7),
    railMuted: const Color(0xFFC6BAAC),
    railLine: const Color(0xFF41444C),
    seam: const [
      BoxShadow(color: Color(0x1A282A2F), offset: Offset(0, 4), blurRadius: 12),
    ],
    seamHover: const [
      BoxShadow(color: Color(0x24282A2F), offset: Offset(0, 8), blurRadius: 20),
    ],
    stack: const [
      BoxShadow(
        color: Color(0x57282A2F),
        offset: Offset(-14, 0),
        blurRadius: 28,
        spreadRadius: -14,
      ),
      BoxShadow(color: Color(0x0F282A2F), spreadRadius: 1),
    ],
  );

  static final darkTheme = AppColors(
    dark: true,
    bgPrimary: const Color(0xFF30333A),
    bgSecondary: const Color(0xFF2A2D33),
    bgTertiary: const Color(0xFF222428),
    bgElevated: const Color(0xFF30333A),
    bgHover: _rgba(255, 255, 255, 0.06),
    bgPressed: _rgba(255, 255, 255, 0.09),
    bgSelectedBlock: _rgba(240, 160, 100, 0.18),
    textSelection: _rgba(240, 160, 100, 0.34),
    textPrimary: const Color(0xFFF3EFE7),
    textSecondary: const Color(0xFFC6BAAC),
    textTertiary: const Color(0xFF999DA6),
    textDisabled: const Color(0xFF575B64),
    iconPrimary: const Color(0xFFF3EFE7),
    iconSecondary: const Color(0xFFC6BAAC),
    iconTertiary: const Color(0xFF999DA6),
    border: const Color(0xFF41444C),
    borderSubtle: const Color(0xFF363940),
    borderStrong: const Color(0xFF575B64),
    borderTranslucent: const Color(0xFF41444C),
    divider: const Color(0xFF41444C),
    accent: const Color(0xFFF0A064),
    accentHover: const Color(0xFFF5B47F),
    inlineCodeBg: const Color(0xFF2A2D33),
    inlineCodeText: const Color(0xFFF0A064),
    overlay: _rgba(0, 0, 0, 0.60),
    desk: const Color(0xFF222428),
    accentText: const Color(0xFFF0A064),
    onAccent: const Color(0xFF282A2F),
    focus: const Color(0xFF7AB7FF),
    thread: const Color(0xFF575B64),
    threadStrong: const Color(0xFF999DA6),
    okBg: _rgba(202, 216, 205, 0.22),
    infoBg: _rgba(203, 213, 223, 0.20),
    attnBg: _rgba(240, 210, 156, 0.20),
    dangerBg: _rgba(229, 177, 170, 0.26),
    pawBg: _rgba(221, 163, 157, 0.22),
    rail: const Color(0xFF141618),
    rail2: const Color(0xFF24262B),
    railInk: const Color(0xFFF3EFE7),
    railMuted: const Color(0xFFC6BAAC),
    railLine: const Color(0xFF2B2E33),
    seam: const [
      BoxShadow(color: Color(0x52000000), offset: Offset(0, 4), blurRadius: 12),
    ],
    seamHover: const [
      BoxShadow(color: Color(0x6B000000), offset: Offset(0, 8), blurRadius: 20),
    ],
    stack: const [
      BoxShadow(
        color: Color(0x99000000),
        offset: Offset(-16, 0),
        blurRadius: 30,
        spreadRadius: -12,
      ),
      BoxShadow(color: Color(0x0DFFFFFF), spreadRadius: 1),
    ],
  );
}

/// One entry of the type scale: size / line height in logical px, weight.
class TypeSpec {
  const TypeSpec(this.size, this.line, this.weight);
  final double size, line;
  final int weight;
}

class TypeScale {
  const TypeScale({
    required this.pageTitle,
    required this.h1,
    required this.h2,
    required this.h3,
    required this.body,
    required this.ui,
    required this.uiSmall,
    required this.code,
  });
  final TypeSpec pageTitle, h1, h2, h3, body, ui, uiSmall, code;

  static const desktop = TypeScale(
    pageTitle: TypeSpec(40, 48, 700),
    h1: TypeSpec(30, 39, 600),
    h2: TypeSpec(24, 31.2, 600),
    h3: TypeSpec(20, 26, 600),
    body: TypeSpec(16, 24, 400),
    ui: TypeSpec(14, 20, 500),
    uiSmall: TypeSpec(12, 16, 500),
    code: TypeSpec(13.6, 20.4, 400),
  );

  static const mobile = TypeScale(
    pageTitle: TypeSpec(28, 33.6, 700),
    h1: TypeSpec(24, 31.2, 600),
    h2: TypeSpec(20, 26, 600),
    h3: TypeSpec(18, 23.4, 600),
    body: TypeSpec(16, 22, 400),
    ui: TypeSpec(15, 20, 500),
    uiSmall: TypeSpec(13, 16, 500),
    code: TypeSpec(13.6, 20.4, 400), // 85% of body, line 1.5
  );
}

FontWeight fw(int w) => FontWeight.values[(w ~/ 100 - 1).clamp(0, 8)];

TextStyle ts(TypeSpec s, Color color, {int? weight, bool mono = false}) =>
    TextStyle(
      inherit: false,
      fontFamily: mono ? monoFamily : sansFamily,
      // Fragment Mono supplies symbols missing from the bundled body font.
      fontFamilyFallback: mono ? monoFallback : [monoFamily],
      fontSize: s.size,
      height: s.line / s.size,
      leadingDistribution: TextLeadingDistribution.even,
      fontWeight: fw(weight ?? s.weight),
      color: color,
      decoration: TextDecoration.none,
    );

TextStyle tsRaw(double size, double line, int weight, Color color) =>
    ts(TypeSpec(size, line, weight), color);

/// Display typography, including the prototype's condensed variable axes.
TextStyle tsDisplay(TypeSpec s, Color color, {int? weight}) {
  final w = weight ?? 640;
  return ts(s, color, weight: w).copyWith(
    fontFamily: displayFamily,
    fontVariations: [
      FontVariation('opsz', s.size.clamp(12.0, 96.0)),
      const FontVariation('wdth', 90),
      FontVariation('wght', w.toDouble()),
    ],
    letterSpacing: -.03 * s.size,
  );
}

class Layout {
  // desktop
  static const contentWidth = 720.0;
  static const textWidth = 708.0;
  static const blockPaddingX = 2.0;
  static const blockRowGap = 2.0;
  static const textBlockHeight = 28.0;
  static const topbarHeight = 44.0;
  static const sidebarWidth = 240.0;
  static const sidebarRowHeight = 30.0;
  static const handleOffset = -44.0;
  // mobile
  static const gutter = 18.0;
  static const keyboardToolbarHeight = 44.0;
  static const minTapTarget = 44.0;
  static const listRowHeight = 44.0;
  static const indent = 24.0;
}

class Radii {
  // Legacy scalar values for specialised editor geometry.
  static const sm = 4.0, md = 6.0, lg = 10.0, sheet = 12.0;
  static const ctl = BorderRadius.only(
    topLeft: Radius.circular(7),
    topRight: Radius.circular(11),
    bottomRight: Radius.circular(6),
    bottomLeft: Radius.circular(9),
  );
  static const field = BorderRadius.only(
    topLeft: Radius.circular(6),
    topRight: Radius.circular(10),
    bottomRight: Radius.circular(6),
    bottomLeft: Radius.circular(8),
  );
  static const panel = BorderRadius.only(
    topLeft: Radius.circular(12),
    topRight: Radius.circular(18),
    bottomRight: Radius.circular(10),
    bottomLeft: Radius.circular(16),
  );
  static const pill = BorderRadius.all(Radius.circular(999));
}

class Motion {
  static const fast = Duration(milliseconds: 150);
  static const standard = Duration(milliseconds: 220);
  static const sheet = Duration(milliseconds: 250);
  static const wiggle = Duration(milliseconds: 200);
  static const ease = Cubic(.2, .8, .2, 1);
  static const settle = Cubic(.22, 1.25, .36, 1);
  static const easing = ease;
  static MotionDurations of(BuildContext context) =>
      MotionDurations(MediaQuery.disableAnimationsOf(context));
}

class MotionDurations {
  const MotionDurations(this.reduced);
  final bool reduced;
  Duration get fast => reduced ? Duration.zero : Motion.fast;
  Duration get standard => reduced ? Duration.zero : Motion.standard;
  Duration get sheet => reduced ? Duration.zero : Motion.sheet;
  Duration get wiggle => reduced ? Duration.zero : Motion.wiggle;
}

/// Desktop: sidebar, top search, selection toolbar, popovers, drag handles.
/// Mobile: stack navigation, bottom bar, keyboard toolbar, bottom sheets.
enum FormFactor {
  desktop,
  mobile;

  /// The one switch between the two layouts: the window's logical width.
  static const breakpoint = 768.0;

  static FormFactor forWidth(double width) =>
      width < breakpoint ? mobile : desktop;
}

class AppTheme extends InheritedWidget {
  const AppTheme({
    super.key,
    required this.colors,
    required this.formFactor,
    required super.child,
  });
  final AppColors colors;
  final FormFactor formFactor;

  bool get mobile => formFactor == FormFactor.mobile;
  TypeScale get type => mobile ? TypeScale.mobile : TypeScale.desktop;

  static AppTheme of(BuildContext context) =>
      context.dependOnInheritedWidgetOfExactType<AppTheme>()!;

  @override
  bool updateShouldNotify(AppTheme old) =>
      old.colors != colors || old.formFactor != formFactor;
}

extension ThemeX on BuildContext {
  AppColors get colors => AppTheme.of(this).colors;
  TypeScale get type => AppTheme.of(this).type;
  FormFactor get formFactor => AppTheme.of(this).formFactor;
  bool get mobile => AppTheme.of(this).mobile;
}
