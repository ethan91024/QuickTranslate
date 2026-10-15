import { useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";

type Settings = {
  api_key: string;
  base_url: string;
  model: string;
  shortcut: string;
  target_language: string;
  source_language: string;
  interface_language: UiLanguage;
};

type UiLanguage = "zh-Hant" | "zh-Hans" | "en" | "ja" | "ko";
type StatusKey = "loadFailed" | "saveFailed" | "listening" | "shortcutSet";
type StatusMessage = { key: StatusKey; shortcut?: string };

const interfaceLanguages: { value: UiLanguage; label: string }[] = [
  { value: "zh-Hant", label: "繁體中文" },
  { value: "zh-Hans", label: "简体中文" },
  { value: "en", label: "English" },
  { value: "ja", label: "日本語" },
  { value: "ko", label: "한국어" },
];

const sourceLanguages = ["auto", "en", "zh-Hant", "zh-Hans", "ja", "ko", "fr", "de", "es"];

const strings: Record<UiLanguage, {
  tagline: string;
  version: string;
  localInfo: string;
  baseUrl: string;
  baseUrlHelp: string;
  model: string;
  shortcut: string;
  listen: string;
  record: string;
  sourceLanguage: string;
  targetLanguage: string;
  interfaceLanguage: string;
  languages: Record<string, string>;
  status: Record<StatusKey, string>;
}> = {
  "zh-Hant": {
    tagline: "快速螢幕翻譯工具",
    version: "版本",
    localInfo: "預設使用本機 Ollama 模型；Base URL 可連接其他相容服務。",
    baseUrl: "服務網址",
    baseUrlHelp: "本機 Ollama 預設值：http://localhost:11434/v1",
    model: "模型",
    shortcut: "快捷鍵設定",
    listen: "請按下鍵盤快捷鍵或任意滑鼠按鍵...",
    record: "錄製",
    sourceLanguage: "原文語言",
    targetLanguage: "譯文語言",
    interfaceLanguage: "介面語言",
    languages: { auto: "自動偵測", en: "英文", "zh-Hant": "繁體中文", "zh-Hans": "簡體中文", ja: "日文", ko: "韓文", fr: "法文", de: "德文", es: "西班牙文" },
    status: { loadFailed: "載入設定失敗", saveFailed: "儲存設定失敗", listening: "請按下鍵盤快捷鍵或任意滑鼠按鍵...", shortcutSet: "快捷鍵已設為 {shortcut}" },
  },
  "zh-Hans": {
    tagline: "快速屏幕翻译工具",
    version: "版本",
    localInfo: "默认使用本机 Ollama 模型；Base URL 可连接其他兼容服务。",
    baseUrl: "服务网址",
    baseUrlHelp: "本机 Ollama 默认值：http://localhost:11434/v1",
    model: "模型",
    shortcut: "快捷键设置",
    listen: "请按下键盘快捷键或任意鼠标按键...",
    record: "录制",
    sourceLanguage: "原文语言",
    targetLanguage: "译文语言",
    interfaceLanguage: "界面语言",
    languages: { auto: "自动检测", en: "英语", "zh-Hant": "繁体中文", "zh-Hans": "简体中文", ja: "日语", ko: "韩语", fr: "法语", de: "德语", es: "西班牙语" },
    status: { loadFailed: "加载设置失败", saveFailed: "保存设置失败", listening: "请按下键盘快捷键或任意鼠标按键...", shortcutSet: "快捷键已设为 {shortcut}" },
  },
  en: {
    tagline: "Fast screen translation",
    version: "Version",
    localInfo: "Uses the local Ollama model by default. Base URL can point to another compatible service.",
    baseUrl: "Service URL",
    baseUrlHelp: "Default local Ollama URL: http://localhost:11434/v1",
    model: "Model",
    shortcut: "Shortcut",
    listen: "Press a keyboard shortcut or any mouse button...",
    record: "Record",
    sourceLanguage: "Source language",
    targetLanguage: "Target language",
    interfaceLanguage: "Interface language",
    languages: { auto: "Detect automatically", en: "English", "zh-Hant": "Traditional Chinese", "zh-Hans": "Simplified Chinese", ja: "Japanese", ko: "Korean", fr: "French", de: "German", es: "Spanish" },
    status: { loadFailed: "Failed to load settings", saveFailed: "Failed to save settings", listening: "Press a keyboard shortcut or any mouse button...", shortcutSet: "Shortcut set to {shortcut}" },
  },
  ja: {
    tagline: "画面をすばやく翻訳",
    version: "バージョン",
    localInfo: "既定ではローカルの Ollama モデルを使用します。Base URL で互換サービスを指定できます。",
    baseUrl: "サービス URL",
    baseUrlHelp: "ローカル Ollama の既定 URL: http://localhost:11434/v1",
    model: "モデル",
    shortcut: "ショートカット設定",
    listen: "キーボードショートカットまたはマウスボタンを押してください...",
    record: "記録",
    sourceLanguage: "原文の言語",
    targetLanguage: "翻訳先の言語",
    interfaceLanguage: "表示言語",
    languages: { auto: "自動検出", en: "英語", "zh-Hant": "繁体字中国語", "zh-Hans": "簡体字中国語", ja: "日本語", ko: "韓国語", fr: "フランス語", de: "ドイツ語", es: "スペイン語" },
    status: { loadFailed: "設定を読み込めませんでした", saveFailed: "設定を保存できませんでした", listening: "キーボードショートカットまたはマウスボタンを押してください...", shortcutSet: "ショートカットを {shortcut} に設定しました" },
  },
  ko: {
    tagline: "빠른 화면 번역 도구",
    version: "버전",
    localInfo: "기본적으로 로컬 Ollama 모델을 사용합니다. Base URL에서 호환 서비스를 지정할 수 있습니다.",
    baseUrl: "서비스 URL",
    baseUrlHelp: "로컬 Ollama 기본 URL: http://localhost:11434/v1",
    model: "모델",
    shortcut: "단축키 설정",
    listen: "키보드 단축키 또는 마우스 버튼을 누르세요...",
    record: "기록",
    sourceLanguage: "원문 언어",
    targetLanguage: "번역 언어",
    interfaceLanguage: "인터페이스 언어",
    languages: { auto: "자동 감지", en: "영어", "zh-Hant": "번체 중국어", "zh-Hans": "간체 중국어", ja: "일본어", ko: "한국어", fr: "프랑스어", de: "독일어", es: "스페인어" },
    status: { loadFailed: "설정을 불러오지 못했습니다", saveFailed: "설정을 저장하지 못했습니다", listening: "키보드 단축키 또는 마우스 버튼을 누르세요...", shortcutSet: "단축키가 {shortcut}(으)로 설정되었습니다" },
  },
};

const modelOptions = [
  "qwen2.5:3b",
  "qwen2.5:7b",
  "llama3.1",
  "mistral",
  "gpt-4o-mini",
];

const defaultSettings: Settings = {
  api_key: "",
  base_url: "http://localhost:11434/v1",
  model: "qwen2.5:3b",
  shortcut: "Ctrl+Shift+T",
  target_language: "zh-Hant",
  source_language: "en",
  interface_language: "zh-Hant",
};

function App() {
  const [settings, setSettings] = useState<Settings>(defaultSettings);
  const [status, setStatus] = useState<StatusMessage | null>(null);
  const [isListeningShortcut, setIsListeningShortcut] = useState(false);
  const saveQueue = useRef<Promise<void>>(Promise.resolve());
  const appVersion = "0.1.4";
  const locale = strings[settings.interface_language] ?? strings["zh-Hant"];

  useEffect(() => {
    loadSettings();
  }, []);

  useEffect(() => {
    if (!isListeningShortcut) {
      return;
    }

    const handleKeyDown = (event: KeyboardEvent) => {
      if (event.repeat) {
        return;
      }

      const key = event.key;
      if (["Control", "Alt", "Shift", "Meta"].includes(key)) {
        return;
      }

      const nextShortcutParts: string[] = [];
      if (event.ctrlKey) nextShortcutParts.push("Ctrl");
      if (event.altKey) nextShortcutParts.push("Alt");
      if (event.shiftKey) nextShortcutParts.push("Shift");
      if (event.metaKey) nextShortcutParts.push("Meta");

      nextShortcutParts.push(key.length === 1 ? key.toUpperCase() : key);
      const nextShortcut = nextShortcutParts.join("+");
      const nextSettings = { ...settings, shortcut: nextShortcut };

      setSettings(nextSettings);
      setStatus({ key: "shortcutSet", shortcut: nextShortcut });
      setIsListeningShortcut(false);
      void saveSettings(nextSettings);
    };

    const handleMouseDown = (event: MouseEvent) => {
      if (event.button < 0 || event.button > 4) {
        return;
      }

      event.preventDefault();
      event.stopPropagation();

      const nextShortcut = `Mouse${event.button + 1}`;
      const nextSettings = { ...settings, shortcut: nextShortcut };

      setSettings(nextSettings);
      setStatus({ key: "shortcutSet", shortcut: nextShortcut });
      setIsListeningShortcut(false);
      void saveSettings(nextSettings);
    };

    window.addEventListener("keydown", handleKeyDown);
    window.addEventListener("mousedown", handleMouseDown, true);
    return () => {
      window.removeEventListener("keydown", handleKeyDown);
      window.removeEventListener("mousedown", handleMouseDown, true);
    };
  }, [isListeningShortcut, settings]);

  async function loadSettings() {
    try {
      const data = await invoke<Settings>("load_settings");
      setSettings({ ...defaultSettings, ...data });
      setStatus(null);
    } catch {
      setStatus({ key: "loadFailed" });
    }
  }

  async function saveSettings(nextSettings: Settings = settings) {
    const saveOperation = saveQueue.current.then(() => invoke<Settings>("save_settings", {
      apiKey: nextSettings.api_key,
      baseUrl: nextSettings.base_url,
      model: nextSettings.model,
      shortcut: nextSettings.shortcut,
      targetLanguage: nextSettings.target_language,
      sourceLanguage: nextSettings.source_language,
      interfaceLanguage: nextSettings.interface_language,
    }));
    saveQueue.current = saveOperation.then(() => undefined, () => undefined);

    try {
      await saveOperation;
      setStatus(null);
    } catch {
      setStatus({ key: "saveFailed" });
    }
  }

  const updateAndSaveSettings = (nextSettings: Settings) => {
    setSettings(nextSettings);
    setStatus(null);
    void saveSettings(nextSettings);
  };

  return (
    <div style={{ padding: 24, maxWidth: 760, margin: "0 auto" }}>
      <h1>QuickTranslate</h1>
      <p>{locale.tagline}</p>
      <div style={{ fontSize: 12, color: "#57606a", marginBottom: 8 }}>{locale.version}: {appVersion}</div>

      <div
        style={{
          display: "grid",
          gap: 12,
          padding: 16,
          border: "1px solid #d0d7de",
          borderRadius: 8,
          background: "#f6f8fa",
          marginBottom: 16,
        }}
      >
        <div
          style={{
            padding: "10px 12px",
            borderRadius: 6,
            background: "#e6ffed",
            border: "1px solid #a7f3d0",
            color: "#166534",
            fontSize: 13,
          }}
        >
          {locale.localInfo}
        </div>

        <label>
          {locale.baseUrl}
          <input
            value={settings.base_url}
            disabled
            style={{ width: "100%", marginTop: 6, padding: 8, color: "#6e7781", background: "#eaeef2", cursor: "not-allowed" }}
          />
          <div style={{ fontSize: 12, color: "#57606a", marginTop: 4 }}>
            {locale.baseUrlHelp}
          </div>
        </label>

        <label>
          {locale.model}
          <select
            value={settings.model}
            disabled
            style={{ width: "100%", marginTop: 6, padding: 8, color: "#6e7781", background: "#eaeef2", cursor: "not-allowed" }}
          >
            {modelOptions.map((model) => (
              <option key={model} value={model}>{model}</option>
            ))}
          </select>
        </label>

        <label>
          {locale.shortcut}
          <div style={{ display: "flex", gap: 8, marginTop: 6 }}>
            <input
              value={settings.shortcut}
              readOnly
              style={{ flex: 1, padding: 8 }}
            />
            <button
              type="button"
              onClick={() => {
                setIsListeningShortcut(true);
                setStatus({ key: "listening" });
              }}
              style={{ minWidth: 90 }}
            >
              {isListeningShortcut ? locale.listen : locale.record}
            </button>
          </div>
        </label>

        <label>
          {locale.sourceLanguage}
          <select
            value={settings.source_language}
            onChange={(event) => updateAndSaveSettings({ ...settings, source_language: event.target.value })}
            style={{ width: "100%", marginTop: 6, padding: 8 }}
          >
            {sourceLanguages.map((language) => (
              <option key={language} value={language}>{locale.languages[language]}</option>
            ))}
          </select>
        </label>

        <label>
          {locale.targetLanguage}
          <select
            value={settings.target_language}
            onChange={(event) => updateAndSaveSettings({ ...settings, target_language: event.target.value })}
            style={{ width: "100%", marginTop: 6, padding: 8 }}
          >
            {sourceLanguages.filter((language) => language !== "auto").map((language) => (
              <option key={language} value={language}>{locale.languages[language]}</option>
            ))}
          </select>
        </label>

        <label>
          {locale.interfaceLanguage}
          <select
            value={settings.interface_language}
            onChange={(event) => updateAndSaveSettings({ ...settings, interface_language: event.target.value as UiLanguage })}
            style={{ width: "100%", marginTop: 6, padding: 8 }}
          >
            {interfaceLanguages.map((language) => (
              <option key={language.value} value={language.value}>{language.label}</option>
            ))}
          </select>
        </label>

        {status && <div>{locale.status[status.key].replace("{shortcut}", status.shortcut ?? "")}</div>}
      </div>
    </div>
  );
}

export default App;