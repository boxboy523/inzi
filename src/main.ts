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

function updateClock() {
  const now = new Date();
  const timeString = now.toLocaleString('ko-KR', {
    year: 'numeric', month: '2-digit', day: '2-digit',
    hour: '2-digit', minute: '2-digit', second: '2-digit', hour12: false
  });
  document.getElementById('clock')!.innerText = timeString;
}
setInterval(updateClock, 1000);

window.openTab = (tabName: string) => {
  document.querySelectorAll('.tab-content').forEach(el => el.classList.remove('active'));
  document.querySelectorAll('.tab-btn').forEach(el => el.classList.remove('active'));
  
  document.getElementById(tabName)?.classList.add('active');
  // 버튼 활성화 로직은 event.target 등을 활용해 추가 가능
};

let currentEditingId: string | null = null; // 현재 수정하려는 input ID

window.requestEdit = (machineId: number, toolNum: number) => {
  currentEditingId = `input-${machineId}-${toolNum}`;
  const modal = document.getElementById('password-modal');
  modal?.classList.remove('hidden');
  (document.getElementById('admin-pw') as HTMLInputElement).value = ''; // 초기화
  document.getElementById('admin-pw')?.focus();
};

window.closeModal = () => {
  document.getElementById('password-modal')?.classList.add('hidden');
  currentEditingId = null;
};

window.checkPassword = async () => {
  const inputPw = (document.getElementById('admin-pw') as HTMLInputElement).value;

  const isValid = await invoke('verify_password', { input: inputPw });

  if (isValid) {
    alert("인증되었습니다. 값을 수정하세요.");
    enableEditMode();
    window.closeModal();
  } else {
    alert("비밀번호가 틀렸습니다.");
  }
};

function enableEditMode() {
  if (!currentEditingId) return;
  const inputEl = document.getElementById(currentEditingId) as HTMLInputElement;

  const oldVal = parseFloat(inputEl.value);
  inputEl.disabled = false;
  inputEl.focus();

  inputEl.onblur = async () => {
    const newVal = parseFloat(inputEl.value);
    if (oldVal !== newVal) {
      const [_, machineStr, toolStr] = currentEditingId!.split('-');

      try {
        await invoke('log_offset_change', {
          machineId: parseInt(machineStr),
          toolNum: parseInt(toolStr),
          oldVal: oldVal,
          newVal: newVal
        });
        alert(`저장 완료: ${oldVal} -> ${newVal}`);
      } catch (e) {
        alert("로그 저장 실패: " + e);
      }
    }
    inputEl.disabled = true; // 다시 잠금
    inputEl.onblur = null;   // 이벤트 제거
  };
}

declare global {
  interface Window {
    openTab: (name: string) => void;
    requestEdit: (m: number, t: number) => void;
    closeModal: () => void;
    checkPassword: () => void;
  }
}
