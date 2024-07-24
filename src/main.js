const { invoke } = window.__TAURI__.tauri;

let messages = [];
let sendLocked = false;
let isAtBottom = true;
function endStream() {
  sendLocked = false;
}

async function call_ai(messagesContainer, messageDiv) {
  let ai_context = await get_context();

  const url = "https://api.groq.com/openai/v1/chat/completions";

  let filteredMessages = [];
  let currentLength = 0;
  let max_chars = 10000;
  for (let i = messages.length - 1; i >= 0 && currentLength < max_chars; i--) {
    const messageContentLength = messages[i].content.length;
    if (currentLength + messageContentLength <= max_chars) {
      filteredMessages.unshift(messages[i]); // Add to the beginning of the array
      currentLength += messageContentLength;
    } else {
      break; // Stop adding more messages
    }
  }
  console.log("len before messages:", filteredMessages.length);

  console.log("len messages:", filteredMessages.length);

  const payload = {
    messages: [{ role: "system", content: ai_context.content }, ...messages],
    model: "llama3-70b-8192",
    temperature: 0.5,
    max_tokens: 512,
    top_p: 1,
    stream: true,
    stop: null,
  };

  try {
    const response = await fetch(url, {
      method: "POST",
      headers: {
        "Content-Type": "application/json",
        Authorization: `Bearer ${ai_context.api_key}`,
      },
      body: JSON.stringify(payload),
    });

    const reader = response.body.getReader();
    const decoder = new TextDecoder();

    let lastBuffer = "";

    let fullMessage = "";

    while (true) {
      const { done, value } = await reader.read();
      if (done) {
        endStream();
        return;
      }

      let chunk = decoder.decode(value, { stream: true });
      chunk = lastBuffer + chunk;
      lastBuffer = "";
      console.log(chunk);

      const pieces = chunk
        .split("data: ")
        .filter((piece) => piece.trim() !== ""); // Filter out any empty strings resulting from split
      pieces.forEach((piece) => {
        if (piece.trim().endsWith("[DONE]")) {
          endStream();

          return;
        }
        if (piece.startsWith("data: ")) {
          piece = piece.substring(6).trim().split("\n")[0];
        }
        try {
          const parsedChunk = JSON.parse(piece);

          if (
            parsedChunk.choices &&
            parsedChunk.choices.length > 0 &&
            parsedChunk.choices[0].delta
          ) {
            const out = parsedChunk.choices[0].delta.content || "";
            fullMessage += out;

            messageDiv.innerHTML = marked.parse(fullMessage);
            messageDiv.querySelectorAll("pre code").forEach((el) => {
              hljs.highlightElement(el);
            });

            console.log(out);
            messages[messages.length - 1].content += out;
            if (isAtBottom) {
              scrollToMax(messagesContainer);
            }
          } else {
            console.log(
              "Received chunk does not contain expected data:",
              parsedChunk
            );
          }
        } catch (error) {
          console.error("NON JSON:", error);
          lastBuffer = piece;
        }
      });
    }
  } catch (error) {
    console.error("Failed to fetch:", error);
    endStream();
  }
}
function convertBRnewLines(str) {
  return str.replace(/\n/g, "<br>");
}
async function get_context() {
  let text = await invoke("get_context", {});
  console.log(text);
  return text;
}
function scrollToMax(element) {
  console.log("SCROLLING MAX");
  element.scrollTo({ top: element.scrollHeight - element.clientHeight });
  isAtBottom = true;
}
window.newChat = async function () {
  messages = [];
  sendLocked = false;
  isAtBottom = true;
  const messagesContainer = document.getElementById("messages-container");
  messagesContainer.innerHTML = "";
  await invoke("new_chat", {});
};
window.settings = async function () {
  await invoke("toggle_settings_window", {});
};

document.addEventListener("DOMContentLoaded", async () => {
  const inputField = document.getElementById("message-input");
  const messagesContainer = document.getElementById("messages-container");

  document.addEventListener("keydown", (event) => {
    // let key = event.which || event.keyCode;
    // if (event.altKey && key == 81) {
    //   console.log("DD");
    // inputField.focus();
    // }
  });

  inputField.addEventListener("input", () => {
    inputField.style.height = "auto";
    inputField.style.height =
      Math.min(inputField.scrollHeight - 17, 180) + "px";
  });
  messagesContainer.addEventListener("scroll", () => {
    const scrollHeight = messagesContainer.scrollHeight;
    const scrollTop = messagesContainer.scrollTop;
    const clientHeight = messagesContainer.clientHeight;

    if (scrollTop + clientHeight >= scrollHeight) {
      isAtBottom = true;
    } else {
      isAtBottom = false;
    }
    console.log(isAtBottom);
  });
  inputField.addEventListener("keydown", async (event) => {
    if (event.key === "Enter" && !event.shiftKey && !sendLocked) {
      event.preventDefault();

      console.log(inputField.value);
      messages = [
        ...messages,
        { role: "user", content: inputField.value },
        { role: "assistant", content: "" },
      ];

      const messageDiv = document.createElement("div");
      messageDiv.className = "user-message";
      messageDiv.innerHTML = inputField.value;
      // let divider = document.createElement("hr");
      // let divider2 = document.createElement("hr");

      // messagesContainer.appendChild(divider);
      messagesContainer.appendChild(messageDiv);

      // messagesContainer.appendChild(divider2);

      const aiMessageDiv = document.createElement("div");
      aiMessageDiv.className = "ai-message";
      messagesContainer.appendChild(aiMessageDiv);

      sendLocked = true;
      inputField.value = "";
      inputField.style.height = "auto";
      if (isAtBottom) {
        scrollToMax(messagesContainer);
      }
      await call_ai(messagesContainer, aiMessageDiv);
    }
  });
});
