import { invoke } from "@tauri-apps/api/core";

// Rust의 MachineConfig 구조체와 일치시키는 인터페이스
interface MachineStatus {
    id: number;
    name: string;
    ip: string;
    port: number;
    connected: boolean;
}

async function initMachineList() {
    const listContainer = document.getElementById("machine-list");
    if (!listContainer) return;

    try {
    // get_machines 대신 get_machine_status 호출
        const machines = await invoke<MachineStatus[]>("get_machine_status");
        listContainer.innerHTML = machines.map((m) => {
            const statusClass = m.connected ? "status-on" : "status-off";
            const statusText = m.connected ? "🟢 온라인" : "🔴 오프라인";

            return `
<div class="machine-card">
<h3>${m.name} (ID: ${m.id})</h3>
<p>IP: ${m.ip} : ${m.port}</p>
<p>상태: <span class="${statusClass}">${statusText}</span></p>
</div>
`}).join("");
    } catch (error) {
        listContainer.innerHTML = `<p style="color:red">장비 상태 로드 실패: ${error}</p>`;
    }
}

async function handleReadOffset() {
    const idInput = document.getElementById("input-machine-id") as HTMLInputElement;
    const toolInput = document.getElementById("input-tool-num") as HTMLInputElement;
    const resultDisplay = document.getElementById("offset-result");

    if (!idInput || !toolInput || !resultDisplay) return;

    const machineId = parseInt(idInput.value);
    const toolNum = parseInt(toolInput.value);

    if (isNaN(machineId) || isNaN(toolNum)) {
        alert("장비 ID와 공구 번호를 올바르게 입력해주세요.");
        return;
    }

    try {
        resultDisplay.innerText = "통신 중...";
        // Rust의 read_tool_offset 커맨드 호출
        const offsetValue = await invoke<number>("read_tool_offset", {
            machineId,
            toolNum,
        });
        resultDisplay.innerText = offsetValue.toFixed(3); // 소수점 3자리 표시
    } catch (error) {
        console.error(error);
        resultDisplay.innerText = "읽기 실패";
        alert(`오프셋 읽기 오류: ${error}`);
    }
}

// 3. 이벤트 리스너 등록
window.addEventListener("DOMContentLoaded", () => {
    initMachineList();

    setInterval(() => {
        initMachineList();
    }, 3000);

    const readBtn = document.getElementById("btn-read-offset");
    if (readBtn) {
        readBtn.addEventListener("click", handleReadOffset);
    }
});
