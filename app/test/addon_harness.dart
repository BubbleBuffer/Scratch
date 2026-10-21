// The host boot entry points for add-on integration tests. Keep production
// add-on imports limited to their own API and the kit; test harnesses supply
// the real shell and native initialization as they do for other app tests.
export 'package:notes_app/shell/addons.dart'
    show registerBuiltins, appAddonContext;
export 'harness.dart';
