; The Windows installer, AniRust-<version>-setup.exe, built with Inno Setup
; from the folder the release stages for the zip:
;
;   iscc /DVersion=1.1.0 /DSource=stage\anirust-1.1.0-windows-x86_64 packaging\windows\anirust.iss
;
; It installs for the user who runs it, without administrator rights, into
; %LOCALAPPDATA%\Programs\AniRust, adds the program to the Start menu and to
; "Installed apps", and installs over an older version in place: the AppId
; below never changes. The program's updater runs it with /VERYSILENT and
; /update=yes, which starts the program again when it is done.

#ifndef Version
  #error Version is not defined: iscc /DVersion=x.y.z
#endif
#ifndef Source
  #define Source "..\..\stage\anirust-" + Version + "-windows-x86_64"
#endif

[Setup]
AppId={{59C3AE0C-ECA3-4C2B-912D-F985FB8B60A6}
AppName=AniRust
AppVersion={#Version}
AppVerName=AniRust {#Version}
AppPublisher=mrfrok
AppPublisherURL=https://github.com/mrFrok/AniRust
AppSupportURL=https://github.com/mrFrok/AniRust/issues
AppUpdatesURL=https://github.com/mrFrok/AniRust/releases
DefaultDirName={localappdata}\Programs\AniRust
DisableProgramGroupPage=yes
DisableDirPage=auto
PrivilegesRequired=lowest
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
LicenseFile=..\..\LICENSE
SetupIconFile=..\icons\anirust.ico
UninstallDisplayIcon={app}\anirust.exe
UninstallDisplayName=AniRust
OutputDir=..\..\dist
OutputBaseFilename=AniRust-{#Version}-setup
Compression=lzma2/ultra64
SolidCompression=yes
WizardStyle=modern
; A running AniRust is closed before its files are replaced, and the updater
; starts it again itself (see [Run]).
CloseApplications=yes
RestartApplications=no
ShowLanguageDialog=auto

[Languages]
Name: "en"; MessagesFile: "compiler:Default.isl"
Name: "ru"; MessagesFile: "compiler:Languages\Russian.isl"

[CustomMessages]
en.RemoveData=Remove AniRust's settings, watch history kept on this computer, downloaded GPU runtimes and cache as well?
ru.RemoveData=Удалить также настройки AniRust, историю просмотра на этом компьютере, скачанные среды для видеокарты и кэш?

[Tasks]
Name: "desktopicon"; Description: "{cm:CreateDesktopIcon}"; GroupDescription: "{cm:AdditionalIcons}"; Flags: unchecked

; The bundled folders are replaced whole: a file an older version shipped and
; this one does not would otherwise stay behind and be loaded.
[InstallDelete]
Type: filesandordirs; Name: "{app}\mlrt"
Type: filesandordirs; Name: "{app}\rife"
Type: filesandordirs; Name: "{app}\vapoursynth"

[Files]
Source: "{#Source}\*"; DestDir: "{app}"; Flags: ignoreversion recursesubdirs createallsubdirs

[Icons]
Name: "{autoprograms}\AniRust"; Filename: "{app}\anirust.exe"
Name: "{autodesktop}\AniRust"; Filename: "{app}\anirust.exe"; Tasks: desktopicon

[Run]
Filename: "{app}\anirust.exe"; Description: "{cm:LaunchProgram,AniRust}"; Flags: nowait postinstall skipifsilent
Filename: "{app}\anirust.exe"; Flags: nowait; Check: StartedByUpdater

[Code]
// The updater passes /update=yes; an installer run by hand never does.
function StartedByUpdater: Boolean;
begin
  Result := WizardSilent and (ExpandConstant('{param:update|no}') = 'yes');
end;

// What the program keeps outside its folder: %APPDATA%\anirust (settings,
// progress, the GPU runtimes it fetched) and %LOCALAPPDATA%\anirust (log,
// cache, compiled networks). Asked about, never removed silently.
procedure CurUninstallStepChanged(CurUninstallStep: TUninstallStep);
begin
  if (CurUninstallStep = usPostUninstall) and not UninstallSilent then
    if MsgBox(CustomMessage('RemoveData'), mbConfirmation, MB_YESNO or MB_DEFBUTTON2) = IDYES then
    begin
      DelTree(ExpandConstant('{userappdata}\anirust'), True, True, True);
      DelTree(ExpandConstant('{localappdata}\anirust'), True, True, True);
    end;
end;
