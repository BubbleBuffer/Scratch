// The app shell: WidgetsApp (no Material), the theme from the tokens
// (following the system brightness unless forced), the form factor from the
// window width, the same tokens for the kit's widgets, and the registered
// commands' keyboard shortcuts.

import 'package:agent_kit/agent_kit.dart' show BbThemeScope;
import 'package:flutter/widgets.dart';

import '../feedback/feedback_host.dart';
import '../registry/command_registry.dart';
import '../ui/app_name.dart';
import '../ui/ui.dart';
import 'addon_panels.dart';
import 'navigation.dart';
import 'page_stack.dart';

class NotesShell extends StatefulWidget {
  const NotesShell({
    super.key,
    required this.home,
    this.forceDark,
    this.navigatorKey,
    this.wrap,
  });
  final Widget home;
  final bool? forceDark;
  final GlobalKey<NavigatorState>? navigatorKey;

  /// Wraps everything below the theme (the app scope goes here).
  final Widget Function(Widget child)? wrap;

  @override
  State<NotesShell> createState() => _NotesShellState();
}

class _NotesShellState extends State<NotesShell> {
  final _stack = PageStackNavigator();
  @override
  void dispose() {
    _stack.dispose();
    super.dispose();
  }

  @override
  Widget build(BuildContext context) {
    final navKey = widget.navigatorKey ?? appNavigatorKey;
    return WidgetsApp(
      title: appName,
      color: AppColors.light.accent,
      navigatorKey: navKey,
      navigatorObservers: [_stack],
      debugShowCheckedModeBanner: false,
      pageRouteBuilder: <T>(RouteSettings settings, WidgetBuilder builder) =>
          PageRouteBuilder<T>(
            settings: settings,
            pageBuilder: (c, a, b) => builder(c),
          ),
      builder: (context, child) {
        final dark =
            widget.forceDark ??
            MediaQuery.platformBrightnessOf(context) == Brightness.dark;
        final colors = dark ? AppColors.darkTheme : AppColors.light;
        final form = FormFactor.forWidth(MediaQuery.sizeOf(context).width);
        final type = form == FormFactor.mobile
            ? TypeScale.mobile
            : TypeScale.desktop;
        Widget app = AppTheme(
          colors: colors,
          formFactor: form,
          child: DefaultTextStyle(
            style: ts(type.body, colors.textPrimary),
            child: ScrollConfiguration(
              behavior: const AppScrollBehavior(),
              child: DefaultSelectionStyle(
                cursorColor: colors.textPrimary,
                selectionColor: colors.textSelection,
                child: ColoredBox(
                  color: colors.desk,
                  child: CallbackShortcuts(
                    bindings: {
                      for (final c in commandRegistry.all)
                        for (final s in c.boundShortcuts)
                          s: () {
                            final ctx = navKey.currentContext;
                            if (ctx != null) commandRegistry.run(c.id, ctx);
                          },
                    },
                    // The kit's widgets (the chat outline) and their sheets
                    // read the same tokens through this scope.
                    child: BbThemeScope(
                      brightness: dark ? Brightness.dark : Brightness.light,
                      formFactor: form,
                      child: FeedbackHost(
                        navigatorKey: navKey,
                        child: AddonPanelShell(
                          child: PageStackChrome(child: child!),
                        ),
                      ),
                    ),
                  ),
                ),
              ),
            ),
          ),
        );
        app = PageStackScope(stack: _stack, child: app);
        if (widget.wrap != null) app = widget.wrap!(app);
        return app;
      },
      home: PageStackHome(child: widget.home),
    );
  }
}
