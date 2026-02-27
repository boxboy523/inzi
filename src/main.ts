import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';

// --- 타입 정의 ---
interface ToolData {
    machine_id: number;
    tool_num: number;
    basic_size: number;
    manual_offset: number;
    offset_rate: number;
    active: boolean;
    avg_gauge: number | null;
    final_offset: number | null;
    current_offset: number;
    previous_offset: number;
    life: number;
    count: number;
}

interface MachineUiState {
    machine_id: number;
    upper_tool: ToolData;
    lower_tool: ToolData;
    batch_size: number;
}

interface OffsetLog {
    timestamp: string;
    old_value: number;
    change_amount: number;
    new_value: number;
    success: boolean;
}

// --- 상태 관리 ---
let machines: MachineUiState[] = [];
let editContext: any = null; // 현재 수정 중인 데이터 컨텍스트

// --- DOM 요소 참조 ---
const tableHead = document.getElementById('table-head')!;
const tableBody = document.getElementById('table-body')!;
const editModal = document.getElementById('edit-modal')!;
const historyModal = document.getElementById('history-modal')!;

// --- 데이터 폴링 및 렌더링 ---
async function fetchState() {
    try {
        machines = await invoke('get_all_machine_states');
        renderTable();
    } catch (e) {
        console.error("상태 갱신 실패:", e);
    }
}

function renderTable() {
    // 1. 헤더 렌더링
    let headHtml = `<tr class="bg-[#00B0F0] text-white font-bold h-12 text-lg">
        <th class="border border-gray-300 w-32 bg-[#00B0F0]">항목</th>`;
    machines.forEach(m => {
        headHtml += `<th class="border border-white w-64">${m.machine_id + 1}호기</th>`;
    });
    headHtml += `</tr>`;
    tableHead.innerHTML = headHtml;

    // 2. 바디 렌더링
    let bodyHtml = '';

    // 기준 경치수
    bodyHtml += `<tr class="bg-[#FFC000] h-10"><td class="bg-[#00B0F0] text-white font-bold border border-white">기준 경치수</td>`;
    machines.forEach(m => {
        bodyHtml += `<td class="border border-gray-400 cursor-pointer hover:bg-yellow-300 transition" 
            data-action="edit" data-id="${m.machine_id}" data-upper="true" data-field="basic_size" data-title="기준 경치수">
            ${m.upper_tool.basic_size.toFixed(3)}
        </td>`;
    });
    bodyHtml += `</tr>`;

    // 평균 경치수
    bodyHtml += `<tr class="bg-[#FFC000] h-10"><td class="bg-[#00B0F0] text-white font-bold border border-white">평균 경치수</td>`;
    machines.forEach(m => {
        bodyHtml += `<td class="border border-gray-400 data-id="${m.machine_id}">
            ${(m.upper_tool.avg_gauge || 0).toFixed(3)}
        </td>`;
    });
    bodyHtml += `</tr>`;

    // 보정 치수 헤더
    bodyHtml += `<tr class="bg-[#00B0F0] text-white font-bold h-8 text-xs align-middle">
        <td rowspan="3" class="border border-white text-sm">보정 치수</td>`;
    machines.forEach(() => {
        bodyHtml += `<td class="border border-white p-0">
            <div class="grid grid-cols-2 h-full align-middle"><div class="border-r border-white/30">자동보정값</div><div>수동보정값</div></div>
        </td>`;
    });
    bodyHtml += `</tr>`;

    // 보정 치수 데이터
    bodyHtml += `<tr class="bg-[#FFC000] h-12">`;
    machines.forEach(m => {
        const autoOffset = (m.upper_tool.basic_size - (m.upper_tool.avg_gauge || 0)).toFixed(3);
        bodyHtml += `<td class="border border-gray-400 p-0">
            <div class="grid grid-cols-2 h-full items-center">
                <div class="border-r border-gray-400 h-full flex items-center justify-center">${autoOffset}</div>
                <div class="h-full flex items-center justify-center bg-[#00B050] text-white font-bold cursor-pointer hover:bg-green-600 m-1 rounded"
                    data-action="edit" data-id="${m.machine_id}" data-upper="true" data-field="manual_offset" data-title="수동 보정값">
                    ${m.upper_tool.manual_offset.toFixed(3)} <br> (INPUT)
                </div>
            </div>
        </td>`;
    });
    bodyHtml += `</tr>`;

    // 최종 보정값
    bodyHtml += `<tr class="bg-[#FFC000] h-8">`;
    machines.forEach(m => {
        const finalVal = (m.upper_tool.basic_size - (m.upper_tool.avg_gauge || 0) + m.upper_tool.manual_offset).toFixed(3);
        bodyHtml += `<td class="border border-gray-400 text-xs font-bold">최종 보정값: ${finalVal}</td>`;
    });
    bodyHtml += `</tr>`;

    // 평균 산출 데이터수
    bodyHtml += `<tr class="bg-[#FFC000] h-10"><td class="bg-[#00B0F0] text-white font-bold border border-white text-xs">평균 산출<br>데이터수</td>`;
    machines.forEach(m => {
        bodyHtml += `<td class="border border-gray-400 cursor-pointer hover:bg-yellow-300" 
            data-action="edit-batch" data-id="${m.machine_id}">
            ${m.batch_size}
        </td>`;
    });
    bodyHtml += `</tr>`;

    // 보정 옵셋 NO
    bodyHtml += `<tr class="bg-[#FFC000]"><td class="bg-[#00B0F0] text-white font-bold border border-white">보정 옵셋 NO.</td>`;
    machines.forEach(m => {
        const upActive = m.upper_tool.active;
        const dnActive = m.lower_tool.active;
        bodyHtml += `<td class="border border-gray-400 p-1">
            <div class="flex justify-center items-center gap-2 mb-1 bg-yellow-200 p-1 rounded">
                <span class="text-xs font-bold cursor-pointer hover:bg-yellow-400 p-0.5 rounded transition"
                      data-action="edit" data-id="${m.machine_id}" data-upper="true" data-field="tool_num" data-title="황삭 툴 번호">
                    황삭(T${m.upper_tool.tool_num})
                </span>
                <button data-action="toggle" data-id="${m.machine_id}" data-upper="true" 
                    class="${upActive ? 'bg-green-600' : 'bg-red-500'} text-white text-xs px-2 py-0.5 rounded shadow">
                    ${upActive ? 'ON' : 'OFF'}
                </button>
                <button data-action="edit" data-id="${m.machine_id}" data-upper="true" data-field="offset_rate" data-title="황삭 보정률"
                    class="bg-blue-800 text-white text-xs px-1 rounded">
                    ${(m.upper_tool.offset_rate * 100).toFixed(0)}%
                </button>
            </div>
            <div class="flex justify-center items-center gap-2 bg-yellow-200 p-1 rounded">
                <span class="text-xs font-bold cursor-pointer hover:bg-yellow-400 p-0.5 rounded transition"
                      data-action="edit" data-id="${m.machine_id}" data-upper="false" data-field="tool_num" data-title="정삭 툴 번호">
                    정삭(T${m.lower_tool.tool_num})
                <button data-action="toggle" data-id="${m.machine_id}" data-upper="false" 
                    class="${dnActive ? 'bg-green-600' : 'bg-red-500'} text-white text-xs px-2 py-0.5 rounded shadow">
                    ${dnActive ? 'ON' : 'OFF'}
                </button>
                <button data-action="edit" data-id="${m.machine_id}" data-upper="false" data-field="offset_rate" data-title="정삭 보정률"
                    class="bg-blue-800 text-white text-xs px-1 rounded">
                    ${(m.lower_tool.offset_rate * 100).toFixed(0)}%
                </button>
            </div>
        </td>`;
    });
    bodyHtml += `</tr>`;

    // 실 보정값
    bodyHtml += `<tr class="bg-[#FFC000] h-10"><td class="bg-[#00B0F0] text-white font-bold border border-white">실 보정값</td>`;
    machines.forEach(m => {
        bodyHtml += `<td class="border border-gray-400">
            <div class="grid grid-cols-2 gap-1 text-xs">
                <div>황: ${(m.upper_tool.final_offset || 0).toFixed(4)}</div>
                <div>정: ${(m.lower_tool.final_offset || 0).toFixed(4)}</div>
            </div>
        </td>`;
    });
    bodyHtml += `</tr>`;

    bodyHtml += `<tr class="bg-[#FFC000] h-10"><td class="bg-[#00B0F0] text-white font-bold border border-white">이전 옵셋</td>`;
    machines.forEach(m => {
        bodyHtml += `<td class="border border-gray-400 p-0">
            <div class="grid grid-cols-2 h-full text-xs">
                <div class="border-r border-gray-400 flex items-center justify-center cursor-pointer hover:bg-yellow-300" 
                     data-action="history" data-id="${m.machine_id}" data-tool="${m.upper_tool.tool_num}">
                    ${m.upper_tool.previous_offset.toFixed(4)}
                </div>
                <div class="flex items-center justify-center cursor-pointer hover:bg-yellow-300" 
                     data-action="history" data-id="${m.machine_id}" data-tool="${m.lower_tool.tool_num}">
                    ${m.lower_tool.previous_offset.toFixed(4)}
                </div>
            </div>
        </td>`;
    });
    bodyHtml += `</tr>`;

    // 현재 옵셋
    bodyHtml += `<tr class="bg-[#FFC000] h-10"><td class="bg-[#00B0F0] text-white font-bold border border-white">현재 옵셋</td>`;
    machines.forEach(m => {
        bodyHtml += `<td class="border border-gray-400 p-0">
            <div class="grid grid-cols-2 h-full text-xs">
                <div class="border-r border-gray-400 flex items-center justify-center cursor-pointer hover:bg-yellow-300" 
                     data-action="history" data-id="${m.machine_id}" data-tool="${m.upper_tool.tool_num}">
                    ${m.upper_tool.current_offset.toFixed(4)}
                </div>
                <div class="flex items-center justify-center cursor-pointer hover:bg-yellow-300" 
                     data-action="history" data-id="${m.machine_id}" data-tool="${m.lower_tool.tool_num}">
                    ${m.lower_tool.current_offset.toFixed(4)}
                </div>
            </div>
        </td>`;
    });
    bodyHtml += `</tr>`;

    // 공구 설정수명 (Life)
    bodyHtml += `<tr class="bg-[#FFC000] h-10"><td class="bg-[#00B0F0] text-white font-bold border border-white">설정 수명</td>`;
    machines.forEach(m => {
        bodyHtml += `<td class="border border-gray-400 p-0">
            <div class="grid grid-cols-2 h-full font-bold">
                <div class="border-r border-gray-400 flex items-center justify-center text-blue-800">${m.upper_tool.life} EA</div>
                <div class="flex items-center justify-center text-blue-800">${m.lower_tool.life} EA</div>
            </div>
        </td>`;
    });
    bodyHtml += `</tr>`;

    // 공구 사용수명 (Count)
    bodyHtml += `<tr class="bg-[#FFC000] h-10"><td class="bg-[#00B0F0] text-white font-bold border border-white">사용 수명</td>`;
    machines.forEach(m => {
        bodyHtml += `<td class="border border-gray-400 p-0">
            <div class="grid grid-cols-2 h-full font-bold">
                <div class="border-r border-gray-400 flex items-center justify-center text-red-700">${m.upper_tool.count} EA</div>
                <div class="flex items-center justify-center text-red-700">${m.lower_tool.count} EA</div>
            </div>
        </td>`;
    });
    bodyHtml += `</tr>`;

    tableBody.innerHTML = bodyHtml;
}

// 에러 팝업 닫기
document.getElementById('btn-error-close')!.addEventListener('click', () => {
    document.getElementById('error-modal')!.classList.add('hidden');
    document.getElementById('error-modal')!.classList.remove('flex');
});

// 시스템 에러 이벤트 리스너 등록
listen<string>('sys-error', (event) => {
    const errorModal = document.getElementById('error-modal')!;
    const errorMsg = document.getElementById('error-message')!;
    
    // 이미 떠있지 않을 때만 띄움 (깜빡임 방지)
    if (errorModal.classList.contains('hidden')) {
        errorMsg.textContent = event.payload;
        errorModal.classList.remove('hidden');
        errorModal.classList.add('flex');
    }
});

// --- 이벤트 위임 (Event Delegation) ---
document.addEventListener('click', async (e) => {
    const target = (e.target as HTMLElement);
    if (!target) return;

    // 1. 가상 키패드 버튼 처리
    if (target.classList.contains('keypad-btn')) {
        const editInput = document.getElementById('edit-input') as HTMLInputElement;
        const key = target.getAttribute('data-key') || target.textContent?.trim() || '';
        
        if (key === 'clear') {
            editInput.value = '0';
        } else if (key === 'backspace' || key === '←') {
            editInput.value = editInput.value.slice(0, -1);

            if (editInput.value === '' || editInput.value === '-') {
                editInput.value = '0';
            }
        } else if (key === 'minus' || key === '+/-') {
            // 부호 반전 기능
            if (editInput.value.startsWith('-')) {
                editInput.value = editInput.value.substring(1);
            } else {
                if (editInput.value !== '0') editInput.value = '-' + editInput.value;
            }
        } else {
            if (editInput.value === '0' && key !== '.') {
                editInput.value = key;
            } else {
                editInput.value += key;
            }
        }
        return;
    }

    // 2. 기존 동작 처리 로직 계속...
    const actionTarget = target.closest('[data-action]');
    if (!actionTarget) return;

    const action = actionTarget.getAttribute('data-action');
    const machineId = Number(actionTarget.getAttribute('data-id'));
    
    const isUpper = target.getAttribute('data-upper') === 'true';
    const machine = machines.find(m => m.machine_id === machineId);
    
    if (!machine) return;

    if (action === 'edit' || action === 'edit-batch') {
        const field = target.getAttribute('data-field') || 'batch_size';
        const title = target.getAttribute('data-title') || '산출 데이터수';
        const tool = isUpper ? machine.upper_tool : machine.lower_tool;
        
        let val = 0;
        if (field === 'batch_size') val = machine.batch_size;
        else if (field === 'offset_rate') val = (tool as any)[field] * 100;
        else val = (tool as any)[field];

        editContext = { machineId, isUpper, field };
        document.getElementById('edit-title')!.textContent = `${machineId}호기 ${title}`;
        (document.getElementById('edit-input') as HTMLInputElement).value = val.toString();
        
        editModal.classList.remove('hidden');
        editModal.classList.add('flex');
    } 
    else if (action === 'toggle') {
        const tool = isUpper ? machine.upper_tool : machine.lower_tool;
        try {
            await invoke('update_tool_settings', {
                machineId, isUpper, active: !tool.active,
                basicSize: null, manualOffset: null, offsetRate: null
            });
            fetchState();
        } catch (err) { alert(err); }
    }
    else if (action === 'history') {
        const toolNum = Number(actionTarget.getAttribute('data-tool'));
        try {
            const logs: OffsetLog[] = await invoke('get_offset_history', { machineId, toolNum, limit: 100 });
            document.getElementById('history-title')!.textContent = `오프셋 수정 이력 (${machineId}호기 - 공구 ${toolNum})`;
            
            const historyBody = document.getElementById('history-body')!;
            // 값을 10000으로 나누어 소수점 4자리 형태로 표시
            historyBody.innerHTML = logs.map(log => `
                <tr class="border-b hover:bg-gray-100">
                    <td class="p-1">${new Date(log.timestamp).toLocaleString()}</td>
                    <td class="p-1">${(log.old_value / 10000).toFixed(4)}</td>
                    <td class="p-1 font-bold ${log.change_amount > 0 ? 'text-red-600' : 'text-blue-600'}">${(log.change_amount / 10000).toFixed(4)}</td>
                    <td class="p-1">${(log.new_value / 10000).toFixed(4)}</td>
                    <td class="p-1">${log.success ? 'O' : 'X'}</td>
                </tr>
            `).join('');

            historyModal.classList.remove('hidden');
            historyModal.classList.add('flex');
        } catch (err) { alert("로그 조회 실패: " + err); }
    }
    else if (action === 'popup-avg') {
        alert(`${machineId}호기 현재 수집된 게이지 데이터 팝업 (구현 필요)`);
    }
});

// --- 모달 버튼 이벤트 ---
document.getElementById('btn-edit-cancel')!.addEventListener('click', () => {
    editModal.classList.add('hidden');
    editModal.classList.remove('flex');
});

document.getElementById('btn-history-close')!.addEventListener('click', () => {
    historyModal.classList.add('hidden');
    historyModal.classList.remove('flex');
});

document.getElementById('btn-edit-save')!.addEventListener('click', async (e) => {
    e.preventDefault();

    if (!editContext) {
        console.error("수정 컨텍스트가 없습니다.");
        return;
    }
    let rawVal = (document.getElementById('edit-input') as HTMLInputElement).value;

    if (rawVal === '' || rawVal === '-') rawVal = '0';

    const inputVal = parseFloat(rawVal);

    try {
        if (editContext.field === 'batch_size') {
            await invoke('update_batch_size', { machineId: editContext.machineId, newSize: Math.floor(inputVal) });
        } else {
            const finalVal = editContext.field === 'offset_rate' ? inputVal / 100.0 : inputVal;
            const args: any = {
                machineId: editContext.machineId,
                isUpper: editContext.isUpper,
                basicSize: null, manualOffset: null, offsetRate: null, active: null, toolNum: null
            };
            
            if (editContext.field === 'basic_size') args.basicSize = finalVal;
            if (editContext.field === 'manual_offset') args.manualOffset = finalVal;
            if (editContext.field === 'offset_rate') args.offsetRate = finalVal;
            if (editContext.field === 'tool_num') args.toolNum = Math.floor(finalVal);

            await invoke('update_tool_settings', args);
        }
        await fetchState();
        editModal.classList.add('hidden');
        editModal.classList.remove('flex');
    } catch (e) {
        alert("저장 실패: " + e);
    }
});

async function initApp() {
    try {
        // 백엔드에서 config.json에 설정된 폰트 크기를 가져와 루트(html)에 적용
        const baseFontSize = await invoke<number>('get_font_size');
        document.documentElement.style.fontSize = `${baseFontSize}px`;
    } catch (e) {
        console.error("폰트 설정 로드 실패:", e);
    }

    // 데이터 로드 및 1초 폴링 시작
    fetchState();
    setInterval(fetchState, 1000);
}

initApp();
