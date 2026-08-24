//! Windows 破坏性目录操作的目标根安全与对象身份。
//!
//! 本模块只读取用户选择的目标根对象，不递归枚举目录内容。调用方得到的是经过 Windows
//! Handle 复验的 Final Path、卷序列号和 128-bit File ID；后续 Preview、Run 和紧邻副作用
//! 的授权检查必须复用同一模型，不能退回字符串或 `Path::exists()` 判断。

use std::error::Error;
use std::ffi::{c_void, OsStr, OsString};
use std::fmt::{Display, Formatter};
use std::mem::{size_of, MaybeUninit};
use std::os::windows::ffi::{OsStrExt, OsStringExt};
use std::path::{Component, Path, PathBuf, Prefix};
use std::ptr::{null, null_mut};

use serde::{Deserialize, Serialize};
use windows_sys::core::GUID;
use windows_sys::Win32::Foundation::{CloseHandle, HANDLE, INVALID_HANDLE_VALUE};
use windows_sys::Win32::Globalization::{CompareStringOrdinal, CSTR_EQUAL};
use windows_sys::Win32::Storage::FileSystem::{
    CreateFileW, FileAttributeTagInfo, FileIdInfo, GetFileInformationByHandleEx,
    GetFinalPathNameByHandleW, FILE_ATTRIBUTE_DIRECTORY, FILE_ATTRIBUTE_REPARSE_POINT,
    FILE_ATTRIBUTE_TAG_INFO, FILE_FLAG_BACKUP_SEMANTICS, FILE_FLAG_OPEN_REPARSE_POINT,
    FILE_ID_INFO, FILE_NAME_NORMALIZED, FILE_READ_ATTRIBUTES, FILE_SHARE_DELETE, FILE_SHARE_READ,
    FILE_SHARE_WRITE, OPEN_EXISTING, VOLUME_NAME_DOS,
};
use windows_sys::Win32::System::Com::CoTaskMemFree;
use windows_sys::Win32::System::SystemInformation::{GetSystemDirectoryW, GetWindowsDirectoryW};
use windows_sys::Win32::UI::Shell::{
    FOLDERID_Desktop, FOLDERID_Documents, FOLDERID_Downloads, FOLDERID_OneDrive, FOLDERID_Profile,
    FOLDERID_ProgramData, FOLDERID_ProgramFiles, FOLDERID_ProgramFilesX64,
    FOLDERID_ProgramFilesX86, FOLDERID_RoamingAppData, SHGetKnownFolderPath, KF_FLAG_DONT_VERIFY,
};

/// DeletePaths Safety Policy 的首个稳定语义版本。
#[allow(dead_code)]
pub(crate) const DELETE_PATH_POLICY_VERSION: u32 = 1;

/// Safety Guard 可稳定分支处理的失败原因。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) enum DeleteSafetyErrorCode {
    /// 请求没有任何目标。
    EmptyTargets,
    /// 输入使用了 destructive Built-in 不接受的 Device Namespace。
    DangerousNamespace,
    /// 输入不是 Windows 绝对目录路径。
    NotAbsolute,
    /// `..` 试图越过路径根。
    EscapesRoot,
    /// 目标在检查时不存在。
    NotFound,
    /// 目标根对象不是目录。
    NotDirectory,
    /// 顶层目标是 Junction、Symbolic Link、Mount Point 或其他 Reparse Point。
    ReparsePoint,
    /// Windows 无法稳定读取目标根对象身份。
    Unavailable,
    /// 输入或 Final Path 命中不可删除的关键根或其受保护子树。
    CriticalPath,
    /// Preview/Run 已绑定的目标根对象在副作用前发生变化。
    TargetChanged,
    /// 系统保护路径集合无法完整建立。
    ProtectedPathsUnavailable,
}

impl DeleteSafetyErrorCode {
    /// 返回不会因 Debug 格式变化而漂移的内部稳定错误码。
    pub(crate) const fn as_str(self) -> &'static str {
        match self {
            Self::EmptyTargets => "emptyTargets",
            Self::DangerousNamespace => "dangerousNamespace",
            Self::NotAbsolute => "notAbsolute",
            Self::EscapesRoot => "escapesRoot",
            Self::NotFound => "notFound",
            Self::NotDirectory => "notDirectory",
            Self::ReparsePoint => "reparsePoint",
            Self::Unavailable => "unavailable",
            Self::CriticalPath => "criticalPath",
            Self::TargetChanged => "targetChanged",
            Self::ProtectedPathsUnavailable => "protectedPathsUnavailable",
        }
    }
}

/// 不携带任意路径文本的稳定 Safety Guard 错误。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct DeleteSafetyError {
    /// 折叠后的目标序号；建立系统根失败时为空。
    pub(crate) target_index: Option<usize>,
    /// 供 Planner、IPC 和测试稳定匹配的原因。
    pub(crate) code: DeleteSafetyErrorCode,
}

impl DeleteSafetyError {
    /// 创建不回显本机路径的目标错误。
    fn target(target_index: usize, code: DeleteSafetyErrorCode) -> Self {
        Self {
            target_index: Some(target_index),
            code,
        }
    }

    /// 创建建立系统保护根时的全局错误。
    fn protected_paths() -> Self {
        Self {
            target_index: None,
            code: DeleteSafetyErrorCode::ProtectedPathsUnavailable,
        }
    }
}

impl Display for DeleteSafetyError {
    /// 只格式化稳定错误码和可选序号，不泄露目标路径。
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        match self.target_index {
            Some(index) => write!(formatter, "{:?}：target[{index}]", self.code),
            None => write!(formatter, "{:?}", self.code),
        }
    }
}

impl Error for DeleteSafetyError {}

/// 一个目录根对象在单台 Windows 主机上的稳定身份。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct PathFingerprint {
    /// 经过词法规范化、供固定 Executor 使用的输入路径。
    pub(crate) normalized_path: PathBuf,
    /// Windows Handle 解析后的 DOS Final Path。
    pub(crate) final_path: PathBuf,
    /// `FILE_ID_INFO` 返回的卷序列号。
    pub(crate) volume_serial_number: u64,
    /// `FILE_ID_INFO` 返回的 128-bit 文件身份。
    pub(crate) file_id: [u8; 16],
    /// 顶层对象是否为 Reparse Point；通过的删除目标始终为 `false`。
    pub(crate) is_reparse_point: bool,
}

/// 一个通过根级 Safety Guard 的删除目标。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct DeleteTarget {
    /// 折叠后进入 Execution Spec 的稳定顺序。
    pub(crate) index: usize,
    /// Preview、Run 和逐目标授权共用的完整对象身份。
    pub(crate) fingerprint: PathFingerprint,
}

/// 当前目标集合需要的用户确认等级。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) enum DeleteRiskDecision {
    /// 普通业务子目录不增加烦扰确认。
    Normal,
    /// 精确命中常用用户根目录，需要后端验证强化确认。
    HighRisk,
}

/// 一次 DeletePaths 检查的完整、确定结果。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct DeleteSafetyReport {
    /// 去重和祖先折叠后真正执行的目标。
    pub(crate) targets: Vec<DeleteTarget>,
    /// 原请求中因重复或被父目录覆盖而不再执行的项数。
    pub(crate) folded_count: usize,
    /// 任一目标 high-risk 时，整个 Execution 都要求强化确认。
    pub(crate) risk: DeleteRiskDecision,
}

/// 由 Windows Known Folder、系统 API 和 Tauri App Data 组成的保护根集合。
#[derive(Debug, Clone)]
pub(crate) struct ProtectedPathSet {
    /// 命中根或任意子孙时都拒绝的系统/应用树。
    critical_trees: Vec<PathBuf>,
    /// 只在精确命中时拒绝的当前用户 Profile 根。
    critical_exact: Vec<PathBuf>,
    /// 只在精确命中时要求强化确认的常用用户根。
    high_risk_exact: Vec<PathBuf>,
}

impl ProtectedPathSet {
    /// 按 Tauri Windows App Data 约定建立 CmdBox 的完整系统保护根。
    pub(crate) fn for_cmdbox() -> Result<Self, DeleteSafetyError> {
        let roaming = query_known_folder(&FOLDERID_RoamingAppData, true)?
            .ok_or_else(DeleteSafetyError::protected_paths)?;
        Self::from_system(roaming.join("com.cmdbox.app"))
    }

    /// 从 Windows API 建立运行主机的保护根；`app_data` 必须是 Tauri 解析的 CmdBox App Data。
    pub(crate) fn from_system(app_data: PathBuf) -> Result<Self, DeleteSafetyError> {
        let windows = query_system_directory(GetWindowsDirectoryW)?;
        let system = query_system_directory(GetSystemDirectoryW)?;
        let profile = query_known_folder(&FOLDERID_Profile, true)?
            .ok_or_else(DeleteSafetyError::protected_paths)?;
        let program_data = query_known_folder(&FOLDERID_ProgramData, true)?
            .ok_or_else(DeleteSafetyError::protected_paths)?;
        let program_files = query_known_folder(&FOLDERID_ProgramFiles, true)?
            .ok_or_else(DeleteSafetyError::protected_paths)?;

        let mut critical_trees = vec![windows, system, program_data, program_files, app_data];
        for identifier in [&FOLDERID_ProgramFilesX64, &FOLDERID_ProgramFilesX86] {
            if let Some(path) = query_known_folder(identifier, false)? {
                push_unique_path(&mut critical_trees, path);
            }
        }

        let mut high_risk_exact = Vec::new();
        for identifier in [
            &FOLDERID_Desktop,
            &FOLDERID_Documents,
            &FOLDERID_Downloads,
            &FOLDERID_OneDrive,
        ] {
            if let Some(path) = query_known_folder(identifier, false)? {
                push_unique_path(&mut high_risk_exact, path);
            }
        }

        Ok(Self {
            critical_trees,
            critical_exact: vec![profile],
            high_risk_exact,
        })
    }

    /// 创建测试和纯策略验证使用的显式保护根，不读取任何系统目录。
    #[cfg(test)]
    pub(crate) fn explicit(
        critical_trees: Vec<PathBuf>,
        critical_exact: Vec<PathBuf>,
        high_risk_exact: Vec<PathBuf>,
    ) -> Self {
        Self {
            critical_trees,
            critical_exact,
            high_risk_exact,
        }
    }

    /// 同时根据用户输入和 Handle Final Path 计算保护等级。
    fn classify(&self, normalized: &Path, final_path: &Path) -> PathClassification {
        if is_volume_or_unc_share_root(normalized) || is_volume_or_unc_share_root(final_path) {
            return PathClassification::Critical;
        }
        for candidate in [normalized, final_path] {
            if self.critical_trees.iter().any(|root| {
                is_same_or_descendant(candidate, root) || is_same_or_descendant(root, candidate)
            }) || self.critical_exact.iter().any(|root| {
                windows_paths_equal(candidate, root) || is_same_or_descendant(root, candidate)
            }) {
                return PathClassification::Critical;
            }
        }
        if [normalized, final_path].iter().any(|candidate| {
            self.high_risk_exact
                .iter()
                .any(|root| windows_paths_equal(candidate, root))
        }) {
            PathClassification::HighRisk
        } else {
            PathClassification::Normal
        }
    }
}

/// 根路径保护判断的内部三态。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PathClassification {
    /// 普通业务目录。
    Normal,
    /// 需要强化确认的精确用户根。
    HighRisk,
    /// 不允许 Built-in destructive command 操作。
    Critical,
}

/// 只持有一个目标根 Handle，并在所有返回路径上关闭它。
struct OwnedHandle(HANDLE);

impl Drop for OwnedHandle {
    /// 关闭 Safety 检查使用的目录 Handle。
    fn drop(&mut self) {
        // SAFETY: `OwnedHandle` 只由成功的 `CreateFileW` 构造，且不会复制或转移底层值。
        unsafe { CloseHandle(self.0) };
    }
}

/// 对原始 Folder[] 执行词法折叠和 Windows 根对象安全检查。
pub(crate) fn inspect_delete_targets(
    values: &[String],
    protected_paths: &ProtectedPathSet,
) -> Result<DeleteSafetyReport, DeleteSafetyError> {
    if values.is_empty() {
        return Err(DeleteSafetyError::target(
            0,
            DeleteSafetyErrorCode::EmptyTargets,
        ));
    }

    let mut effective = Vec::<PathBuf>::new();
    for (input_index, value) in values.iter().enumerate() {
        let normalized = normalize_delete_path(value)
            .map_err(|code| DeleteSafetyError::target(input_index, code))?;
        if effective
            .iter()
            .any(|existing| windows_paths_equal(existing, &normalized))
            || effective
                .iter()
                .any(|existing| is_same_or_descendant(&normalized, existing))
        {
            continue;
        }
        effective.retain(|existing| !is_same_or_descendant(existing, &normalized));
        effective.push(normalized);
    }

    let folded_count = values.len().saturating_sub(effective.len());
    let mut risk = DeleteRiskDecision::Normal;
    let mut targets = Vec::with_capacity(effective.len());
    for (index, path) in effective.into_iter().enumerate() {
        let fingerprint = inspect_root(index, path)?;
        match protected_paths.classify(&fingerprint.normalized_path, &fingerprint.final_path) {
            PathClassification::Critical => {
                return Err(DeleteSafetyError::target(
                    index,
                    DeleteSafetyErrorCode::CriticalPath,
                ));
            }
            PathClassification::HighRisk => risk = DeleteRiskDecision::HighRisk,
            PathClassification::Normal => {}
        }
        targets.push(DeleteTarget { index, fingerprint });
    }

    Ok(DeleteSafetyReport {
        targets,
        folded_count,
        risk,
    })
}

/// 在紧邻单个删除副作用前，按已验证路径重新打开目标根并比较完整对象身份。
///
/// 本入口不接受 side channel 路径，也不递归扫描目录；Executor 只能按可信 index 取回
/// Preview/Run 已绑定的 Fingerprint，再由此路径执行 fresh root check。
#[allow(dead_code)] // CMD04-SESSION-01 接入 Delete Executor 后由生产链路调用。
pub(crate) fn revalidate_delete_target(
    index: usize,
    expected: &PathFingerprint,
    protected_paths: &ProtectedPathSet,
) -> Result<(), DeleteSafetyError> {
    let actual = inspect_root(index, expected.normalized_path.clone())?;
    if protected_paths.classify(&actual.normalized_path, &actual.final_path)
        == PathClassification::Critical
    {
        return Err(DeleteSafetyError::target(
            index,
            DeleteSafetyErrorCode::CriticalPath,
        ));
    }
    if &actual != expected {
        return Err(DeleteSafetyError::target(
            index,
            DeleteSafetyErrorCode::TargetChanged,
        ));
    }
    Ok(())
}

/// 规范化 destructive Built-in 接受的 Windows 绝对路径语法。
fn normalize_delete_path(value: &str) -> Result<PathBuf, DeleteSafetyErrorCode> {
    if value.is_empty() || value.contains('\0') {
        return Err(DeleteSafetyErrorCode::Unavailable);
    }
    let folded = value
        .chars()
        .map(|character| if character == '/' { '\\' } else { character })
        .collect::<String>()
        .to_ascii_lowercase();
    if folded.starts_with(r"\\.\")
        || folded.starts_with(r"\??\")
        || folded.starts_with(r"\\?\globalroot\")
    {
        return Err(DeleteSafetyErrorCode::DangerousNamespace);
    }
    let path = Path::new(value);
    if !path.is_absolute() {
        return Err(DeleteSafetyErrorCode::NotAbsolute);
    }
    if matches!(
        path.components().next(),
        Some(Component::Prefix(prefix))
            if matches!(prefix.kind(), Prefix::DeviceNS(_) | Prefix::Verbatim(_))
    ) {
        return Err(DeleteSafetyErrorCode::DangerousNamespace);
    }

    let mut normalized = PathBuf::new();
    let mut normal_components = 0_usize;
    for component in path.components() {
        match component {
            Component::Prefix(prefix) => normalized.push(prefix.as_os_str()),
            Component::RootDir => normalized.push(component.as_os_str()),
            Component::CurDir => {}
            Component::ParentDir => {
                if normal_components == 0 {
                    return Err(DeleteSafetyErrorCode::EscapesRoot);
                }
                normalized.pop();
                normal_components -= 1;
            }
            Component::Normal(part) => {
                normalized.push(part);
                normal_components += 1;
            }
        }
    }
    Ok(normalized)
}

/// 通过不跟随顶层 Reparse Point 的目录 Handle 读取属性、Final Path 和 128-bit 身份。
fn inspect_root(
    target_index: usize,
    normalized_path: PathBuf,
) -> Result<PathFingerprint, DeleteSafetyError> {
    let wide = wide_null(normalized_path.as_os_str());
    // SAFETY: `wide` 以 NUL 结尾；Security Attributes 和 template handle 均按 Win32 契约为空。
    let raw_handle = unsafe {
        CreateFileW(
            wide.as_ptr(),
            FILE_READ_ATTRIBUTES,
            FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
            null(),
            OPEN_EXISTING,
            FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT,
            null_mut(),
        )
    };
    if raw_handle == INVALID_HANDLE_VALUE {
        let code = match std::io::Error::last_os_error().kind() {
            std::io::ErrorKind::NotFound => DeleteSafetyErrorCode::NotFound,
            _ => DeleteSafetyErrorCode::Unavailable,
        };
        return Err(DeleteSafetyError::target(target_index, code));
    }
    let handle = OwnedHandle(raw_handle);

    let attributes = query_handle_value::<FILE_ATTRIBUTE_TAG_INFO>(
        handle.0,
        FileAttributeTagInfo,
        target_index,
    )?;
    if attributes.FileAttributes & FILE_ATTRIBUTE_DIRECTORY == 0 {
        return Err(DeleteSafetyError::target(
            target_index,
            DeleteSafetyErrorCode::NotDirectory,
        ));
    }
    if attributes.FileAttributes & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
        return Err(DeleteSafetyError::target(
            target_index,
            DeleteSafetyErrorCode::ReparsePoint,
        ));
    }

    let identity = query_handle_value::<FILE_ID_INFO>(handle.0, FileIdInfo, target_index)?;
    let final_path = query_final_path(handle.0, target_index)?;
    Ok(PathFingerprint {
        normalized_path,
        final_path,
        volume_serial_number: identity.VolumeSerialNumber,
        file_id: identity.FileId.Identifier,
        is_reparse_point: false,
    })
}

/// 读取固定尺寸的 `GetFileInformationByHandleEx` 结构。
fn query_handle_value<T>(
    handle: HANDLE,
    information_class: i32,
    target_index: usize,
) -> Result<T, DeleteSafetyError> {
    let mut value = MaybeUninit::<T>::zeroed();
    // SAFETY: `value` 指向 `size_of::<T>()` 可写字节，class 与调用处传入的 T 一一对应。
    let success = unsafe {
        GetFileInformationByHandleEx(
            handle,
            information_class,
            value.as_mut_ptr().cast::<c_void>(),
            size_of::<T>() as u32,
        )
    };
    if success == 0 {
        return Err(DeleteSafetyError::target(
            target_index,
            DeleteSafetyErrorCode::Unavailable,
        ));
    }
    // SAFETY: Win32 已成功初始化完整的固定尺寸输出结构。
    Ok(unsafe { value.assume_init() })
}

/// 读取 Handle 对应的 DOS Final Path，并去除 Win32 Extended 前缀供统一比较。
fn query_final_path(handle: HANDLE, target_index: usize) -> Result<PathBuf, DeleteSafetyError> {
    // 先用空 buffer 查询所需 UTF-16 单元数。
    let required = unsafe {
        GetFinalPathNameByHandleW(
            handle,
            null_mut(),
            0,
            FILE_NAME_NORMALIZED | VOLUME_NAME_DOS,
        )
    };
    if required == 0 {
        return Err(DeleteSafetyError::target(
            target_index,
            DeleteSafetyErrorCode::Unavailable,
        ));
    }
    let mut buffer = vec![0_u16; required as usize + 1];
    // SAFETY: buffer 容量按上一次调用返回值分配，指针在调用期间有效且可写。
    let written = unsafe {
        GetFinalPathNameByHandleW(
            handle,
            buffer.as_mut_ptr(),
            buffer.len() as u32,
            FILE_NAME_NORMALIZED | VOLUME_NAME_DOS,
        )
    };
    if written == 0 || written as usize >= buffer.len() {
        return Err(DeleteSafetyError::target(
            target_index,
            DeleteSafetyErrorCode::Unavailable,
        ));
    }
    let raw = OsString::from_wide(&buffer[..written as usize]);
    Ok(strip_extended_prefix(Path::new(&raw)))
}

/// 把 `\\?\C:\...` 和 `\\?\UNC\...` 转回可比较、可展示的 DOS/UNC 路径。
fn strip_extended_prefix(path: &Path) -> PathBuf {
    let encoded = path.as_os_str().encode_wide().collect::<Vec<_>>();
    let unc_prefix = r"\\?\UNC\".encode_utf16().collect::<Vec<_>>();
    if utf16_ascii_starts_with_ignore_case(&encoded, &unc_prefix) {
        let mut normalized = vec![b'\\' as u16, b'\\' as u16];
        normalized.extend_from_slice(&encoded[unc_prefix.len()..]);
        return PathBuf::from(OsString::from_wide(&normalized));
    }
    let extended_prefix = r"\\?\".encode_utf16().collect::<Vec<_>>();
    if utf16_ascii_starts_with_ignore_case(&encoded, &extended_prefix) {
        return PathBuf::from(OsString::from_wide(&encoded[extended_prefix.len()..]));
    }
    path.to_path_buf()
}

/// 对固定 ASCII Win32 前缀执行不分大小写的 UTF-16 比较，不改写其余路径单元。
fn utf16_ascii_starts_with_ignore_case(value: &[u16], prefix: &[u16]) -> bool {
    value.len() >= prefix.len()
        && value
            .iter()
            .zip(prefix)
            .all(|(actual, expected)| ascii_utf16_lower(*actual) == ascii_utf16_lower(*expected))
}

/// 只折叠 ASCII `A..Z`，供固定 Win32 前缀比较；其余 UTF-16 单元原样保留。
const fn ascii_utf16_lower(value: u16) -> u16 {
    if value >= b'A' as u16 && value <= b'Z' as u16 {
        value + (b'a' - b'A') as u16
    } else {
        value
    }
}

/// 调用一个返回 Windows 目录文本的 Kernel32 API，并处理动态 buffer。
fn query_system_directory(
    query: unsafe extern "system" fn(*mut u16, u32) -> u32,
) -> Result<PathBuf, DeleteSafetyError> {
    let mut buffer = vec![0_u16; 32_768];
    // SAFETY: buffer 在调用期间有效且可写，容量小于 u32 上限。
    let written = unsafe { query(buffer.as_mut_ptr(), buffer.len() as u32) };
    if written == 0 || written as usize >= buffer.len() {
        return Err(DeleteSafetyError::protected_paths());
    }
    Ok(PathBuf::from(OsString::from_wide(
        &buffer[..written as usize],
    )))
}

/// 调用 Shell Known Folder API；可选目录不可用时返回 `None`，必需目录交由调用方拒绝。
fn query_known_folder(
    identifier: &GUID,
    required: bool,
) -> Result<Option<PathBuf>, DeleteSafetyError> {
    let mut raw = null_mut::<u16>();
    // SAFETY: `raw` 是有效输出指针；当前用户 token 按 API 契约使用 null。
    let result = unsafe {
        SHGetKnownFolderPath(identifier, KF_FLAG_DONT_VERIFY as u32, null_mut(), &mut raw)
    };
    if result < 0 || raw.is_null() {
        if required {
            return Err(DeleteSafetyError::protected_paths());
        }
        return Ok(None);
    }
    let mut length = 0_usize;
    // SAFETY: 成功的 SHGetKnownFolderPath 返回 NUL 结尾、由 CoTaskMemFree 释放的 UTF-16。
    unsafe {
        while *raw.add(length) != 0 {
            length += 1;
        }
    }
    // SAFETY: 上面的扫描只读取 API 返回的 NUL 结尾字符串。
    let value = unsafe { OsString::from_wide(std::slice::from_raw_parts(raw, length)) };
    // SAFETY: `raw` 的所有权来自 SHGetKnownFolderPath，且只释放一次。
    unsafe { CoTaskMemFree(raw.cast::<c_void>()) };
    Ok(Some(PathBuf::from(value)))
}

/// 向根集合加入一个按 Windows ordinal ignore-case 唯一的路径。
fn push_unique_path(paths: &mut Vec<PathBuf>, candidate: PathBuf) {
    if !paths
        .iter()
        .any(|existing| windows_paths_equal(existing, &candidate))
    {
        paths.push(candidate);
    }
}

/// 判断两个 Windows 路径是否按 ordinal ignore-case 完全相等。
fn windows_paths_equal(first: &Path, second: &Path) -> bool {
    windows_os_equal(first.as_os_str(), second.as_os_str())
}

/// 判断 candidate 是否等于 root 或位于 root 子树中，不依赖当前 Locale。
fn is_same_or_descendant(candidate: &Path, root: &Path) -> bool {
    let candidate_components = candidate.components().collect::<Vec<_>>();
    let root_components = root.components().collect::<Vec<_>>();
    root_components.len() <= candidate_components.len()
        && root_components.iter().zip(candidate_components.iter()).all(
            |(root_component, candidate_component)| {
                windows_os_equal(root_component.as_os_str(), candidate_component.as_os_str())
            },
        )
}

/// 使用 `CompareStringOrdinal(..., TRUE)` 比较两个 UTF-16 Windows 名称。
fn windows_os_equal(first: &OsStr, second: &OsStr) -> bool {
    let first = first.encode_wide().collect::<Vec<_>>();
    let second = second.encode_wide().collect::<Vec<_>>();
    // SAFETY: 两个 slice 指针在调用期间有效，显式长度避免依赖 NUL 结尾。
    unsafe {
        CompareStringOrdinal(
            first.as_ptr(),
            first.len() as i32,
            second.as_ptr(),
            second.len() as i32,
            1,
        ) == CSTR_EQUAL
    }
}

/// 识别 `C:\`、`\\server\share\` 及其 Extended 等价形式。
fn is_volume_or_unc_share_root(path: &Path) -> bool {
    let normalized = strip_extended_prefix(path);
    let mut components = normalized.components();
    let Some(Component::Prefix(prefix)) = components.next() else {
        return false;
    };
    match prefix.kind() {
        Prefix::UNC(_, _) | Prefix::VerbatimUNC(_, _) => match components.next() {
            None => true,
            Some(Component::RootDir) => components.next().is_none(),
            Some(_) => false,
        },
        Prefix::Disk(_) | Prefix::VerbatimDisk(_) | Prefix::Verbatim(_) | Prefix::DeviceNS(_) => {
            matches!(components.next(), Some(Component::RootDir)) && components.next().is_none()
        }
    }
}

/// 把 OsStr 转成传给 Win32 的 NUL 结尾 UTF-16。
fn wide_null(value: &OsStr) -> Vec<u16> {
    value.encode_wide().chain(std::iter::once(0)).collect()
}

#[cfg(test)]
mod tests {
    //! 目标根身份、保护分级、路径折叠与不递归安全边界测试。

    use std::fs;
    use std::ops::Deref;
    use std::path::{Path, PathBuf};
    use std::process::Command;

    use super::{
        inspect_delete_targets, is_same_or_descendant, revalidate_delete_target,
        windows_paths_equal, DeleteRiskDecision, DeleteSafetyErrorCode, ProtectedPathSet,
    };

    /// 创建只属于当前测试的隔离根；测试不会把现有目录作为删除目标。
    fn isolated_root(label: &str) -> IsolatedRoot {
        IsolatedRoot(
            std::env::temp_dir()
                .join("CmdBox")
                .join(format!("safety-{label}-{}", uuid::Uuid::new_v4())),
        )
    }

    /// 断言失败时也只清理当前测试创建的 UUID 隔离根。
    struct IsolatedRoot(PathBuf);

    impl Deref for IsolatedRoot {
        type Target = Path;

        /// 让测试夹具可直接使用 `Path` 的只读和构造方法。
        fn deref(&self) -> &Self::Target {
            &self.0
        }
    }

    impl AsRef<Path> for IsolatedRoot {
        /// 供 `std::fs` 泛型入口使用当前 UUID 根。
        fn as_ref(&self) -> &Path {
            &self.0
        }
    }

    impl Drop for IsolatedRoot {
        /// 删除当前夹具独占的 UUID 根；不存在时保持幂等。
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    /// 确保测试内部 Junction 在隔离根递归清理前先被当作链接本身移除。
    struct JunctionGuard(PathBuf);

    impl Drop for JunctionGuard {
        /// 只移除当前测试创建的 Junction 根对象，不触碰其指向目录。
        fn drop(&mut self) {
            let _ = fs::remove_dir(&self.0);
        }
    }

    /// 在两个 UUID 隔离路径之间创建不要求 Symlink 特权的目录 Junction。
    fn create_junction(link: &Path, destination: &Path) -> JunctionGuard {
        let status = Command::new("cmd.exe")
            .args([
                "/D",
                "/C",
                "mklink",
                "/J",
                link.to_string_lossy().as_ref(),
                destination.to_string_lossy().as_ref(),
            ])
            .status()
            .expect("应启动固定的本地 Junction 创建命令");
        assert!(status.success(), "隔离测试 Junction 应创建成功");
        JunctionGuard(link.to_path_buf())
    }

    /// 创建不命中任何真实系统根的显式测试策略。
    fn policy(root: &Path) -> ProtectedPathSet {
        ProtectedPathSet::explicit(
            vec![root.join("critical-tree")],
            vec![root.join("critical-exact")],
            vec![root.join("high-risk")],
        )
    }

    /// 验证 Windows ordinal ignore-case 路径比较和祖先关系不受大小写影响。
    #[test]
    fn compares_windows_paths_and_descendants_case_insensitively() {
        assert!(windows_paths_equal(
            Path::new(r"C:\Users\Example"),
            Path::new(r"c:\users\example")
        ));
        assert!(is_same_or_descendant(
            Path::new(r"C:\Users\Example\Child"),
            Path::new(r"c:\users\example")
        ));
        assert!(!is_same_or_descendant(
            Path::new(r"C:\Users\Example2"),
            Path::new(r"C:\Users\Example")
        ));
    }

    /// 验证 DOS、UNC 和 Extended UNC 根在有无尾分隔符时都不能进入删除目标。
    #[test]
    fn recognizes_volume_and_unc_share_roots_with_or_without_trailing_separator() {
        for root in [
            r"C:\",
            r"\\server\share",
            r"\\server\share\",
            r"\\?\UNC\server\share",
            r"\\?\UNC\server\share\",
        ] {
            assert!(
                super::is_volume_or_unc_share_root(Path::new(root)),
                "必须识别根路径：{root}"
            );
        }
        assert!(!super::is_volume_or_unc_share_root(Path::new(
            r"\\server\share\child"
        )));
    }

    /// 验证重复项和父子项只保留真正需要执行的父目录。
    #[test]
    fn removes_duplicates_and_collapses_descendants() {
        let root = isolated_root("collapse");
        let parent = root.join("Parent");
        let child = parent.join("Child");
        fs::create_dir_all(&child).expect("应创建隔离父子目录");

        let report = inspect_delete_targets(
            &[
                child.to_string_lossy().into_owned(),
                parent.to_string_lossy().into_owned(),
                parent.to_string_lossy().to_uppercase(),
            ],
            &policy(&root),
        )
        .expect("父目录应通过检查");

        assert_eq!(report.targets.len(), 1);
        assert_eq!(report.folded_count, 2);
        assert!(windows_paths_equal(
            &report.targets[0].fingerprint.normalized_path,
            &parent
        ));
        fs::remove_dir_all(&root).expect("应清理隔离测试根");
    }

    /// 验证目录对象被删除重建后，即使路径文本相同，128-bit 身份也发生变化。
    #[test]
    fn fingerprints_directory_identity_and_detects_recreation() {
        let root = isolated_root("identity");
        let target = root.join("中文 target");
        fs::create_dir_all(&target).expect("应创建隔离目标");
        let protected = policy(&root);
        let first = inspect_delete_targets(&[target.to_string_lossy().into_owned()], &protected)
            .expect("首次身份检查应成功")
            .targets
            .remove(0)
            .fingerprint;
        revalidate_delete_target(0, &first, &protected).expect("未变化目标应通过紧邻副作用复验");
        fs::remove_dir(&target).expect("只删除当前测试创建的空目标");
        fs::create_dir(&target).expect("应在相同路径重建隔离目标");
        let changed = revalidate_delete_target(0, &first, &protected)
            .expect_err("同名重建目标必须被紧邻副作用复验拒绝");
        assert_eq!(changed.code, DeleteSafetyErrorCode::TargetChanged);
        let second = inspect_delete_targets(&[target.to_string_lossy().into_owned()], &protected)
            .expect("重建后身份检查应成功")
            .targets
            .remove(0)
            .fingerprint;

        assert_eq!(first.normalized_path, second.normalized_path);
        assert_ne!(
            (first.volume_serial_number, first.file_id),
            (second.volume_serial_number, second.file_id)
        );
        fs::remove_dir_all(&root).expect("应清理隔离测试根");
    }

    /// 验证顶层 Junction/Symlink 在 Handle 属性检查中直接被拒绝。
    #[test]
    fn rejects_top_level_reparse_point() {
        let root = isolated_root("reparse");
        let destination = root.join("destination");
        let link = root.join("link");
        fs::create_dir_all(&destination).expect("应创建链接目标");
        let link_guard = create_junction(&link, &destination);

        let error = inspect_delete_targets(&[link.to_string_lossy().into_owned()], &policy(&root))
            .expect_err("顶层 Reparse Point 必须被拒绝");
        assert_eq!(error.code, DeleteSafetyErrorCode::ReparsePoint);
        drop(link_guard);
        fs::remove_dir_all(&root).expect("应清理隔离测试根");
    }

    /// 验证根级 Guard 不因内部 Junction 或深层内容而递归扫描和拒绝整个目标。
    #[test]
    fn checks_only_the_target_root_without_enumerating_descendants() {
        let root = isolated_root("root-only");
        let target = root.join("target");
        let sentinel = root.join("outside-sentinel");
        let deep_leaf = target.join("a").join("b").join("c");
        let internal_link = target.join("internal-junction");
        fs::create_dir_all(&deep_leaf).expect("应创建隔离深层目录");
        fs::create_dir_all(&sentinel).expect("应创建隔离 sentinel");
        fs::write(sentinel.join("keep.txt"), b"keep").expect("应创建 sentinel 文件");
        let link_guard = create_junction(&internal_link, &sentinel);

        let report =
            inspect_delete_targets(&[target.to_string_lossy().into_owned()], &policy(&root))
                .expect("内部内容和 Junction 不应触发递归 Safety 扫描");
        assert_eq!(report.targets.len(), 1);
        assert!(sentinel.join("keep.txt").is_file());

        drop(link_guard);
        fs::remove_dir_all(&root).expect("应清理隔离测试根");
    }

    /// 验证 critical 树、critical 精确根和 high-risk 精确根的分级边界。
    #[test]
    fn classifies_protected_and_high_risk_roots_without_blocking_children_of_profile() {
        let root = isolated_root("protected");
        let critical_tree = root.join("critical-tree");
        let critical_child = critical_tree.join("child");
        let critical_exact = root.join("critical-exact");
        let allowed_profile_child = critical_exact.join("child");
        let high_risk = root.join("high-risk");
        let normal = root.join("normal");
        for path in [
            &critical_child,
            &critical_exact,
            &allowed_profile_child,
            &high_risk,
            &normal,
        ] {
            fs::create_dir_all(path).expect("应创建隔离分级目录");
        }
        let protected = policy(&root);

        let ancestor_error =
            inspect_delete_targets(&[root.to_string_lossy().into_owned()], &protected)
                .expect_err("包含任一 critical 根的祖先目录必须被拒绝");
        assert_eq!(ancestor_error.code, DeleteSafetyErrorCode::CriticalPath);

        for path in [&critical_tree, &critical_child, &critical_exact] {
            let error = inspect_delete_targets(&[path.to_string_lossy().into_owned()], &protected)
                .expect_err("critical 目标必须被拒绝");
            assert_eq!(error.code, DeleteSafetyErrorCode::CriticalPath);
        }
        let allowed = inspect_delete_targets(
            &[allowed_profile_child.to_string_lossy().into_owned()],
            &protected,
        )
        .expect("Profile 子目录本身不应因精确根策略被拒绝");
        assert_eq!(allowed.risk, DeleteRiskDecision::Normal);
        let warning =
            inspect_delete_targets(&[high_risk.to_string_lossy().into_owned()], &protected)
                .expect("高风险精确根应进入强化确认而非直接拒绝");
        assert_eq!(warning.risk, DeleteRiskDecision::HighRisk);

        fs::remove_dir_all(&root).expect("应清理隔离测试根");
    }

    /// 验证危险 Namespace、相对路径、缺失目录、文件和卷根都 fail-closed。
    #[test]
    fn rejects_invalid_missing_file_and_volume_root_targets() {
        let root = isolated_root("invalid");
        fs::create_dir_all(&root).expect("应创建隔离测试根");
        let file = root.join("file.txt");
        fs::write(&file, b"fixture").expect("应创建隔离测试文件");
        let protected = policy(&root);
        let cases = vec![
            (
                r"\\.\C:\danger".to_owned(),
                DeleteSafetyErrorCode::DangerousNamespace,
            ),
            (
                "//./C:/danger".to_owned(),
                DeleteSafetyErrorCode::DangerousNamespace,
            ),
            (
                r"\\?\gLoBaLrOoT\Device\HarddiskVolume1".to_owned(),
                DeleteSafetyErrorCode::DangerousNamespace,
            ),
            (
                "//?/GLOBALROOT/Device/HarddiskVolume1".to_owned(),
                DeleteSafetyErrorCode::DangerousNamespace,
            ),
            ("relative".to_owned(), DeleteSafetyErrorCode::NotAbsolute),
            (
                root.join("missing").to_string_lossy().into_owned(),
                DeleteSafetyErrorCode::NotFound,
            ),
            (
                file.to_string_lossy().into_owned(),
                DeleteSafetyErrorCode::NotDirectory,
            ),
            (r"C:\".to_owned(), DeleteSafetyErrorCode::CriticalPath),
        ];

        for (value, expected) in cases {
            let error = inspect_delete_targets(std::slice::from_ref(&value), &protected)
                .expect_err("非法或危险目标必须被拒绝");
            assert_eq!(error.code, expected, "目标：{value}");
        }
        fs::remove_dir_all(&root).expect("应清理隔离测试根");
    }

    /// 验证系统保护根可以通过 Windows API 建立且至少包含关键和高风险路径。
    #[test]
    fn loads_system_protected_paths_from_windows_apis() {
        let app_data = isolated_root("app-data");
        let protected = ProtectedPathSet::from_system(app_data.to_path_buf())
            .expect("Windows Known Folder 和系统目录 API 应可用");

        assert!(protected.critical_trees.len() >= 5);
        assert_eq!(protected.critical_exact.len(), 1);
        assert!(protected.high_risk_exact.len() >= 3);
        for root in &protected.critical_trees {
            assert_eq!(
                protected.classify(root, root),
                super::PathClassification::Critical
            );
            assert_eq!(
                protected.classify(&root.join("child"), &root.join("child")),
                super::PathClassification::Critical
            );
        }
        for root in &protected.critical_exact {
            assert_eq!(
                protected.classify(root, root),
                super::PathClassification::Critical
            );
            assert_ne!(
                protected.classify(&root.join("child"), &root.join("child")),
                super::PathClassification::Critical
            );
        }
        for root in &protected.high_risk_exact {
            assert_eq!(
                protected.classify(root, root),
                super::PathClassification::HighRisk
            );
            assert_eq!(
                protected.classify(&root.join("child"), &root.join("child")),
                super::PathClassification::Normal
            );
        }
    }
}
