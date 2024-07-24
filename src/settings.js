const { invoke } = window.__TAURI__.tauri;

document.addEventListener("DOMContentLoaded", async () => {
  const savedSettings = await invoke("get_settings");
  console.log(savedSettings);

  document.getElementById("groqApiKey").value = savedSettings.groq_api_key;
  document.getElementById("screenContext").checked =
    savedSettings.screen_context;
  document.getElementById("audioContext").checked = savedSettings.audio_context;

  const updateAllSettings = async () => {
    const groqApiKey = document.getElementById("groqApiKey").value;
    const screenContext = document.getElementById("screenContext").checked;
    const audioContext = document.getElementById("audioContext").checked;

    await invoke("update_settings", {
      settings: {
        // Adjusted to match the expected argument structure
        groq_api_key: groqApiKey,
        screen_context: screenContext,
        audio_context: audioContext,
      },
    });
  };

  // Attach the updateAllSettings function to each input and checkbox
  document
    .getElementById("groqApiKey")
    .addEventListener("input", updateAllSettings);
  document
    .getElementById("screenContext")
    .addEventListener("change", updateAllSettings);
  document
    .getElementById("audioContext")
    .addEventListener("change", updateAllSettings);
});
