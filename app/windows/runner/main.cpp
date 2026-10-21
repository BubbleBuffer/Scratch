#include <flutter/dart_project.h>
#include <flutter/flutter_view_controller.h>
#include <windows.h>

#include <string>

#include "app_links/app_links_plugin_c_api.h"
#include "flutter_window.h"
#include "utils.h"

// Sets one string value under HKEY_CURRENT_USER\<subkey>, creating the key.
// A null |name| sets the key's default value.
static void SetUserString(const wchar_t* subkey, const wchar_t* name,
                          const std::wstring& value) {
  HKEY key = nullptr;
  if (::RegCreateKeyExW(HKEY_CURRENT_USER, subkey, 0, nullptr,
                        REG_OPTION_NON_VOLATILE, KEY_WRITE, nullptr, &key,
                        nullptr) != ERROR_SUCCESS) {
    return;
  }
  ::RegSetValueExW(key, name, 0, REG_SZ,
                   reinterpret_cast<const BYTE*>(value.c_str()),
                   static_cast<DWORD>((value.size() + 1) * sizeof(wchar_t)));
  ::RegCloseKey(key);
}

// Registers the notes:// scheme for the current user, pointing at this exe.
// Runs on every start and rewrites the same values, so a moved build fixes
// itself and no installer is needed.
static void RegisterNotesScheme() {
  wchar_t exe[MAX_PATH];
  const DWORD length = ::GetModuleFileNameW(nullptr, exe, MAX_PATH);
  if (length == 0 || length >= MAX_PATH) return;
  const std::wstring exe_path(exe);
  const std::wstring command = L"\"" + exe_path + L"\" \"%1\"";
  const std::wstring icon = L"\"" + exe_path + L"\",0";

  const wchar_t* scheme = L"Software\\Classes\\notes";
  const std::wstring scheme_key = scheme;
  SetUserString(scheme, nullptr, L"URL:Notes page link");
  SetUserString(scheme, L"URL Protocol", L"");
  SetUserString((scheme_key + L"\\DefaultIcon").c_str(), nullptr, icon);
  SetUserString((scheme_key + L"\\shell\\open\\command").c_str(), nullptr,
                command);
}

int APIENTRY wWinMain(_In_ HINSTANCE instance, _In_opt_ HINSTANCE prev,
                      _In_ wchar_t *command_line, _In_ int show_command) {
  RegisterNotesScheme();

  // Forward a notes:// link to the running instance and foreground it, then
  // exit. This also covers a plain launch while an instance is open.
  if (SendAppLinkToInstance()) {
    return EXIT_SUCCESS;
  }

  // Attach to console when present (e.g., 'flutter run') or create a
  // new console when running with a debugger.
  if (!::AttachConsole(ATTACH_PARENT_PROCESS) && ::IsDebuggerPresent()) {
    CreateAndAttachConsole();
  }

  // Initialize COM, so that it is available for use in the library and/or
  // plugins.
  ::CoInitializeEx(nullptr, COINIT_APARTMENTTHREADED);

  flutter::DartProject project(L"data");

  std::vector<std::string> command_line_arguments =
      GetCommandLineArguments();

  project.set_dart_entrypoint_arguments(std::move(command_line_arguments));

  FlutterWindow window(project);
  Win32Window::Point origin(10, 10);
  Win32Window::Size size(1280, 720);
  if (!window.Create(L"Scratch", origin, size)) {
    return EXIT_FAILURE;
  }
  window.SetQuitOnClose(true);

  ::MSG msg;
  while (::GetMessage(&msg, nullptr, 0, 0)) {
    ::TranslateMessage(&msg);
    ::DispatchMessage(&msg);
  }

  ::CoUninitialize();
  return EXIT_SUCCESS;
}
