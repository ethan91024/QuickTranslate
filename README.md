QuickTranslate (畫面偵測翻譯)

開發緣由: 
我很愛聽英文歌跟看美劇，還有一些英文小說或報導，可是有時會出現我沒看過的生難字詞，
如果是可選取的文字還可以直接翻譯，但是圖片或影片出現的就要手打比較麻煩，簡單來說就是因為懶想要方便自己才開發的，
所以想做一個可以綁在滑鼠側鍵上，用鼠標指到的地方來進行畫面偵測，並在旁邊盡快地產生翻譯。

使用技術:
React 、TypeScript、Tauri 使用者介面
Tesseract OCR 辨識文字
Ollama 本機模型翻譯
Rust 處理螢幕擷取、系統快捷鍵和桌面視窗等原生功能

目前進度: 
目前還只是功能原型／開發中版本。
主要翻譯流程實作：快捷鍵觸發 → 擷取游標附近畫面 → OCR → 找出游標附近的字 → 翻譯 → 浮動視窗顯示。
另有系統匣、快捷鍵設定、來源／目標語言選擇，以及繁中、簡中、英文、日文、韓文語言切換。

待做事項:
翻譯精準度和速度有待加強
加上不同詞意、相關變化和動詞、形容詞等詞類

<img width="639" height="751" alt="image" src="https://github.com/user-attachments/assets/3f4f5b21-29a4-40b9-ab06-3031e5274e95" />

<img width="414" height="111" alt="image" src="https://github.com/user-attachments/assets/d85fa690-c8e4-44f6-a15a-946c1c5e4365" />


