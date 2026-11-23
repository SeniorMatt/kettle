//! Which interface versions exist and in which order their methods sit in the C++ vtable.
//!
//! Slot order is the order of the `virtual` declarations in Valve's published `isteam*.h` headers of each version
//! (compared by reading the headers' method order only; the names below are our own spelling and nothing else of
//! the headers is used). Private entries that Valve hides with `STEAM_PRIVATE_API` still take a slot. A game
//! compiled against version N calls slot k of that version's table, so a wrong order calls the wrong function.
//!
//! The names are matched to implementations in `iface.rs`; a name without one gets the do-nothing function.
//! Pure data: the tests in this file run without loading anything.

use std::ops::RangeInclusive;

/// The interfaces we know the layout of.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Kind {
    Client,
    User,
    Friends,
    Utils,
    Apps,
    UserStats,
    /// ISteamInput: a stub whose `Init`/`Shutdown` say "ok".
    Input,
    /// Any other Steamworks interface we recognise by name but do not implement: a table of do-nothing slots.
    Stub,
}

fn num_after(ver: &str, prefix: &str) -> Option<u32> {
    let rest = ver.strip_prefix(prefix)?;
    if rest.is_empty() || !rest.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    rest.parse().ok()
}

type Row = (&'static str, Kind, RangeInclusive<u32>);

/// Interfaces with a slot table of their own. Ranges are the versions whose layout we have compared.
pub const IMPLEMENTED: [Row; 7] = [
    ("SteamClient", Kind::Client, 17..=23),
    ("SteamUser", Kind::User, 17..=23),
    ("SteamFriends", Kind::Friends, 15..=18),
    ("SteamUtils", Kind::Utils, 7..=11),
    ("STEAMAPPS_INTERFACE_VERSION", Kind::Apps, 6..=9),
    ("STEAMUSERSTATS_INTERFACE_VERSION", Kind::UserStats, 11..=13),
    ("SteamInput", Kind::Input, 1..=7),
];

/// Interfaces games may ask for that we answer with a do-nothing object (and a log line, once).
pub const STUBS: [(&str, RangeInclusive<u32>); 23] = [
    ("SteamMatchMaking", 1..=9),
    ("SteamMatchMakingServers", 1..=3),
    ("SteamNetworking", 1..=6),
    ("SteamNetworkingSockets", 1..=13),
    ("SteamNetworkingMessages", 1..=2),
    ("SteamNetworkingUtils", 1..=4),
    ("STEAMREMOTESTORAGE_INTERFACE_VERSION", 1..=16),
    ("STEAMSCREENSHOTS_INTERFACE_VERSION", 1..=3),
    ("STEAMHTTP_INTERFACE_VERSION", 1..=3),
    ("STEAMUGC_INTERFACE_VERSION", 1..=21),
    ("STEAMINVENTORY_INTERFACE_V", 1..=3),
    ("SteamController", 1..=8),
    ("SteamGameServer", 1..=15),
    ("SteamGameServerStats", 1..=2),
    ("STEAMAPPTICKET_INTERFACE_VERSION", 1..=1),
    ("SteamParties", 1..=2),
    ("SteamRemotePlay", 1..=3),
    ("STEAMMUSIC_INTERFACE_VERSION", 1..=1),
    ("STEAMVIDEO_INTERFACE_V", 1..=3),
    ("STEAMPARENTALSETTINGS_INTERFACE_VERSION", 1..=1),
    ("STEAMGAMESEARCH_INTERFACE_VERSION", 1..=1),
    ("STEAMAPPLIST_INTERFACE_VERSION", 1..=1),
    ("STEAMUNIFIEDMESSAGES_INTERFACE_VERSION", 1..=1),
];

/// The kind and version number of a version string, `None` if we have never heard of it.
pub fn classify(ver: &str) -> Option<(Kind, u32)> {
    for (prefix, kind, range) in IMPLEMENTED {
        if let Some(n) = num_after(ver, prefix)
            && range.contains(&n)
        {
            return Some((kind, n));
        }
    }
    for (prefix, range) in STUBS {
        if let Some(n) = num_after(ver, prefix)
            && range.contains(&n)
        {
            return Some((Kind::Stub, n));
        }
    }
    None
}

fn words(s: &'static str) -> Vec<&'static str> {
    s.split_whitespace().collect()
}

const CLIENT: &str = "CreateSteamPipe BReleaseSteamPipe ConnectToGlobalUser CreateLocalUser ReleaseUser GetISteamUser \
    GetISteamGameServer SetLocalIPBinding GetISteamFriends GetISteamUtils GetISteamMatchmaking \
    GetISteamMatchmakingServers GetISteamGenericInterface GetISteamUserStats GetISteamGameServerStats GetISteamApps \
    GetISteamNetworking GetISteamRemoteStorage GetISteamScreenshots RunFrame GetIPCCallCount SetWarningMessageHook \
    BShutdownIfAllPipesClosed GetISteamHTTP GetISteamUnifiedMessages GetISteamController GetISteamUGC \
    GetISteamAppList GetISteamMusic GetISteamMusicRemote GetISteamHTMLSurface SetPostAPIResultInProcess \
    RemovePostAPIResultInProcess SetCheckCallbackRegisteredInProcess GetISteamInventory GetISteamVideo \
    GetISteamParentalSettings";

fn client_slots(n: u32) -> Vec<&'static str> {
    let mut v = words(CLIENT); // SteamClient017
    if n >= 18 {
        v.insert(19, "GetISteamGameSearch");
        v.extend(["GetISteamInput", "GetISteamParties"]);
    }
    if n >= 19 {
        v.push("GetISteamRemotePlay");
    }
    if n >= 20 {
        v.push("DestroyAllInterfaces");
    }
    if n >= 21 {
        v.retain(|s| *s != "GetISteamAppList" && *s != "GetISteamUnifiedMessages");
    }
    if n >= 22 {
        v.retain(|s| *s != "GetISteamGameSearch");
    }
    if n >= 23 {
        v.retain(|s| *s != "GetISteamMusicRemote");
    }
    v
}

const USER: &str = "GetHSteamUser BLoggedOn GetSteamID InitiateGameConnection TerminateGameConnection \
    TrackAppUsageEvent GetUserDataFolder StartVoiceRecording StopVoiceRecording GetAvailableVoice GetVoice \
    DecompressVoice GetVoiceOptimalSampleRate GetAuthSessionTicket BeginAuthSession EndAuthSession CancelAuthTicket \
    UserHasLicenseForApp BIsBehindNAT AdvertiseGame RequestEncryptedAppTicket GetEncryptedAppTicket GetGameBadgeLevel \
    GetPlayerSteamLevel";

fn user_slots(n: u32) -> Vec<&'static str> {
    let mut v = words(USER); // SteamUser017
    if n >= 18 {
        v.push("RequestStoreAuthURL");
    }
    if n >= 19 {
        v.extend(["BIsPhoneVerified", "BIsTwoFactorEnabled", "BIsPhoneIdentifying", "BIsPhoneRequiringVerification"]);
    }
    if n >= 20 {
        v.extend(["GetMarketEligibility", "GetDurationControl"]);
    }
    if n >= 21 {
        v.push("BSetDurationControlOnlineState");
    }
    if n >= 23 {
        v.insert(14, "GetAuthTicketForWebApi");
    }
    v
}

const FRIENDS: &str = "GetPersonaName SetPersonaName GetPersonaState GetFriendCount GetFriendByIndex \
    GetFriendRelationship GetFriendPersonaState GetFriendPersonaName GetFriendGamePlayed GetFriendPersonaNameHistory \
    GetFriendSteamLevel GetPlayerNickname GetFriendsGroupCount GetFriendsGroupIDByIndex GetFriendsGroupName \
    GetFriendsGroupMembersCount GetFriendsGroupMembersList HasFriend GetClanCount GetClanByIndex GetClanName \
    GetClanTag GetClanActivityCounts DownloadClanActivityCounts GetFriendCountFromSource GetFriendFromSourceByIndex \
    IsUserInSource SetInGameVoiceSpeaking ActivateGameOverlay ActivateGameOverlayToUser ActivateGameOverlayToWebPage \
    ActivateGameOverlayToStore SetPlayedWith ActivateGameOverlayInviteDialog GetSmallFriendAvatar \
    GetMediumFriendAvatar GetLargeFriendAvatar RequestUserInformation RequestClanOfficerList GetClanOwner \
    GetClanOfficerCount GetClanOfficerByIndex GetUserRestrictions SetRichPresence ClearRichPresence \
    GetFriendRichPresence GetFriendRichPresenceKeyCount GetFriendRichPresenceKeyByIndex RequestFriendRichPresence \
    InviteUserToGame GetCoplayFriendCount GetCoplayFriend GetFriendCoplayTime GetFriendCoplayGame JoinClanChatRoom \
    LeaveClanChatRoom GetClanChatMemberCount GetChatMemberByIndex SendClanChatMessage GetClanChatMessage \
    IsClanChatAdmin IsClanChatWindowOpenInSteam OpenClanChatWindowInSteam CloseClanChatWindowInSteam \
    SetListenForFriendsMessages ReplyToFriendMessage GetFriendMessage GetFollowerCount IsFollowing \
    EnumerateFollowingList IsClanPublic IsClanOfficialGameGroup";

fn friends_slots(n: u32) -> Vec<&'static str> {
    let mut v = words(FRIENDS); // SteamFriends015
    if n >= 16 {
        v.push("GetNumChatsWithUnreadPriorityMessages");
    }
    if n >= 17 {
        v.extend([
            "ActivateGameOverlayRemotePlayTogetherInviteDialog",
            "RegisterProtocolInOverlayBrowser",
            "ActivateGameOverlayInviteDialogConnectString",
            "RequestEquippedProfileItems",
            "BHasEquippedProfileItem",
            "GetProfileItemPropertyString",
            "GetProfileItemPropertyUint",
        ]);
    }
    if n >= 18 {
        v.retain(|s| *s != "SetPersonaName" && *s != "GetUserRestrictions");
    }
    v
}

const UTILS: &str = "GetSecondsSinceAppActive GetSecondsSinceComputerActive GetConnectedUniverse GetServerRealTime \
    GetIPCountry GetImageSize GetImageRGBA GetCSERIPPort GetCurrentBatteryPower GetAppID SetOverlayNotificationPosition \
    IsAPICallCompleted GetAPICallFailureReason GetAPICallResult RunFrame GetIPCCallCount SetWarningMessageHook \
    IsOverlayEnabled BOverlayNeedsPresent CheckFileSignature ShowGamepadTextInput GetEnteredGamepadTextLength \
    GetEnteredGamepadTextInput GetSteamUILanguage IsSteamRunningInVR SetOverlayNotificationInset \
    IsSteamInBigPictureMode StartVRDashboard IsVRHeadsetStreamingEnabled SetVRHeadsetStreamingEnabled \
    IsSteamChinaLauncher InitFilterText FilterText GetIPv6ConnectivityState";

fn utils_slots(n: u32) -> Vec<&'static str> {
    let mut v = words(UTILS); // SteamUtils009 (34 slots)
    match n {
        ..=7 => v.truncate(26),
        8 => v.truncate(28),
        _ => {}
    }
    if n >= 10 {
        v.extend([
            "IsSteamRunningOnSteamDeck",
            "ShowFloatingGamepadTextInput",
            "SetGameLauncherMode",
            "DismissFloatingGamepadTextInput",
            "DismissGamepadTextInput",
        ]);
    }
    if n >= 11 {
        v.retain(|s| *s != "GetCSERIPPort" && *s != "IsSteamRunningOnSteamDeck");
        v.extend(["IsRunningOnSteamHardware", "GetSteamHardwareDefaultConfig", "IsRunningUnderProton"]);
    }
    v
}

const APPS: &str = "BIsSubscribed BIsLowViolence BIsCybercafe BIsVACBanned GetCurrentGameLanguage \
    GetAvailableGameLanguages BIsSubscribedApp BIsDlcInstalled GetEarliestPurchaseUnixTime \
    BIsSubscribedFromFreeWeekend GetDLCCount BGetDLCDataByIndex InstallDLC UninstallDLC RequestAppProofOfPurchaseKey \
    GetCurrentBetaName MarkContentCorrupt GetInstalledDepots GetAppInstallDir BIsAppInstalled GetAppOwner \
    GetLaunchQueryParam";

fn apps_slots(n: u32) -> Vec<&'static str> {
    let mut v = words(APPS); // up to GetLaunchQueryParam: 22 slots
    if n == 6 {
        v.push("RegisterActivationCode");
        return v;
    }
    v.extend(["GetDlcDownloadProgress", "GetAppBuildId"]); // SteamApps007
    if n >= 8 {
        v.extend([
            "RequestAllProofOfPurchaseKeys",
            "GetFileDetails",
            "GetLaunchCommandLine",
            "BIsSubscribedFromFamilySharing",
            "BIsTimedTrial",
            "SetDlcContext",
            "GetNumBetas",
            "GetBetaInfo",
            "SetActiveBeta",
        ]);
    }
    if n >= 9 {
        v.extend(["SetGamePerformanceSetting", "SetGameRenderResolution"]);
    }
    v
}

const USER_STATS: &str = "RequestCurrentStats GetStatInt GetStatFloat SetStatInt SetStatFloat UpdateAvgRateStat \
    GetAchievement SetAchievement ClearAchievement GetAchievementAndUnlockTime StoreStats GetAchievementIcon \
    GetAchievementDisplayAttribute IndicateAchievementProgress GetNumAchievements GetAchievementName RequestUserStats \
    GetUserStatInt GetUserStatFloat GetUserAchievement GetUserAchievementAndUnlockTime ResetAllStats \
    FindOrCreateLeaderboard FindLeaderboard GetLeaderboardName GetLeaderboardEntryCount GetLeaderboardSortMethod \
    GetLeaderboardDisplayType DownloadLeaderboardEntries DownloadLeaderboardEntriesForUsers \
    GetDownloadedLeaderboardEntry UploadLeaderboardScore AttachLeaderboardUGC GetNumberOfCurrentPlayers \
    RequestGlobalAchievementPercentages GetMostAchievedAchievementInfo GetNextMostAchievedAchievementInfo \
    GetAchievementAchievedPercent RequestGlobalStats GetGlobalStatInt GetGlobalStatDouble GetGlobalStatHistoryInt \
    GetGlobalStatHistoryDouble";

fn user_stats_slots(n: u32) -> Vec<&'static str> {
    let mut v = words(USER_STATS); // up to GetGlobalStatHistory: 43 slots
    if n == 11 {
        v.extend(["InstallPS3Trophies", "GetTrophySpaceRequiredBeforeInstall", "SetUserStatsData", "GetUserStatsData"]);
    } else {
        v.extend(["GetAchievementProgressLimitsInt", "GetAchievementProgressLimitsFloat"]);
    }
    if n >= 13 {
        v.remove(0); // RequestCurrentStats is gone: Steam sends the stats without being asked
    }
    v
}

/// ISteamInput: the method names we give meaning to (the rest of its 48 slots do nothing).
const INPUT: &str = "Init Shutdown";

/// The method names of an interface in vtable order. Stubs have no names (the table is all do-nothing).
pub fn slots(kind: Kind, n: u32) -> Vec<&'static str> {
    match kind {
        Kind::Client => client_slots(n),
        Kind::User => user_slots(n),
        Kind::Friends => friends_slots(n),
        Kind::Utils => utils_slots(n),
        Kind::Apps => apps_slots(n),
        Kind::UserStats => user_stats_slots(n),
        Kind::Input => words(INPUT),
        Kind::Stub => Vec::new(),
    }
}

/// Does `GetISteamUserStats` of this version send the stats on its own (no `RequestCurrentStats` slot)?
pub fn stats_are_pushed(n: u32) -> bool {
    n >= 13
}

#[cfg(test)]
mod tests {
    use super::*;

    fn count(kind: Kind, n: u32) -> usize {
        slots(kind, n).len()
    }

    fn at(kind: Kind, n: u32, name: &str) -> usize {
        slots(kind, n).iter().position(|s| *s == name).unwrap_or_else(|| panic!("{kind:?}{n} has no {name}"))
    }

    // The counts and positions below were read off the published headers of each version.

    #[test]
    fn client_layouts() {
        let counts = [(17, 37), (18, 40), (19, 41), (20, 42), (21, 40), (22, 39), (23, 38)];
        for (n, c) in counts {
            assert_eq!(count(Kind::Client, n), c, "SteamClient{n:03}");
            assert_eq!(at(Kind::Client, n, "GetISteamUtils"), 9, "the one getter with fewer arguments");
            assert_eq!(at(Kind::Client, n, "GetISteamScreenshots"), 18);
        }
        // RunFrame sits before GetIPCCallCount, and BShutdownIfAllPipesClosed two slots after.
        assert_eq!((at(Kind::Client, 17, "RunFrame"), at(Kind::Client, 17, "BShutdownIfAllPipesClosed")), (19, 22));
        assert_eq!((at(Kind::Client, 18, "RunFrame"), at(Kind::Client, 18, "BShutdownIfAllPipesClosed")), (20, 23));
        assert_eq!(at(Kind::Client, 22, "BShutdownIfAllPipesClosed"), 22);
        assert_eq!(at(Kind::Client, 23, "BShutdownIfAllPipesClosed"), 22);
        assert_eq!(at(Kind::Client, 21, "GetISteamInput"), 36);
        assert_eq!(at(Kind::Client, 23, "GetISteamInput"), 34);
        assert_eq!(*slots(Kind::Client, 23).last().unwrap(), "DestroyAllInterfaces");
    }

    #[test]
    fn user_layouts() {
        let counts = [(17, 24), (18, 25), (19, 29), (20, 31), (21, 32), (22, 32), (23, 33)];
        for (n, c) in counts {
            assert_eq!(count(Kind::User, n), c, "SteamUser{n:03}");
            assert_eq!(at(Kind::User, n, "GetAuthSessionTicket"), 13);
            assert_eq!(at(Kind::User, n, "GetSteamID"), 2);
        }
        assert_eq!(at(Kind::User, 22, "BeginAuthSession"), 14);
        assert_eq!(at(Kind::User, 23, "GetAuthTicketForWebApi"), 14);
        assert_eq!(at(Kind::User, 23, "BeginAuthSession"), 15);
        assert_eq!(at(Kind::User, 23, "RequestEncryptedAppTicket"), 21);
        assert_eq!(at(Kind::User, 22, "RequestEncryptedAppTicket"), 20);
    }

    #[test]
    fn friends_layouts() {
        let counts = [(15, 72), (16, 73), (17, 80), (18, 78)];
        for (n, c) in counts {
            assert_eq!(count(Kind::Friends, n), c, "SteamFriends{n:03}");
        }
        // 018 dropped SetPersonaName and GetUserRestrictions: everything after them moved up.
        assert_eq!(at(Kind::Friends, 17, "GetFriendCount"), 3);
        assert_eq!(at(Kind::Friends, 18, "GetFriendCount"), 2);
        assert_eq!(at(Kind::Friends, 17, "SetRichPresence"), 43);
        assert_eq!(at(Kind::Friends, 18, "SetRichPresence"), 41);
    }

    #[test]
    fn utils_layouts() {
        let counts = [(7, 26), (8, 28), (9, 34), (10, 39), (11, 40)];
        for (n, c) in counts {
            assert_eq!(count(Kind::Utils, n), c, "SteamUtils{n:03}");
        }
        assert_eq!(at(Kind::Utils, 10, "GetAppID"), 9);
        assert_eq!(at(Kind::Utils, 11, "GetAppID"), 8, "011 has no GetCSERIPPort");
        assert_eq!(at(Kind::Utils, 10, "GetSteamUILanguage"), 23);
        assert_eq!(at(Kind::Utils, 11, "GetSteamUILanguage"), 22);
        assert_eq!(*slots(Kind::Utils, 11).last().unwrap(), "IsRunningUnderProton");
    }

    #[test]
    fn apps_layouts() {
        let counts = [(6, 23), (7, 24), (8, 33), (9, 35)];
        for (n, c) in counts {
            assert_eq!(count(Kind::Apps, n), c, "SteamApps{n:03}");
            assert_eq!(at(Kind::Apps, n, "GetAppInstallDir"), 18);
        }
        assert_eq!(at(Kind::Apps, 8, "GetAppBuildId"), 23);
        assert_eq!(at(Kind::Apps, 8, "GetLaunchCommandLine"), 26);
    }

    #[test]
    fn user_stats_layouts() {
        let counts = [(11, 47), (12, 45), (13, 44)];
        for (n, c) in counts {
            assert_eq!(count(Kind::UserStats, n), c, "UserStats{n:03}");
        }
        assert_eq!(at(Kind::UserStats, 12, "StoreStats"), 10);
        assert_eq!(at(Kind::UserStats, 13, "StoreStats"), 9, "013 has no RequestCurrentStats");
        assert!(stats_are_pushed(13) && !stats_are_pushed(12));
    }

    #[test]
    fn version_strings() {
        assert_eq!(classify("SteamClient021"), Some((Kind::Client, 21)));
        assert_eq!(classify("SteamClient023"), Some((Kind::Client, 23)));
        assert_eq!(classify("SteamClient024"), None, "newer than anything compared");
        assert_eq!(classify("SteamUtils011"), Some((Kind::Utils, 11)));
        assert_eq!(classify("STEAMAPPS_INTERFACE_VERSION009"), Some((Kind::Apps, 9)));
        assert_eq!(classify("STEAMUSERSTATS_INTERFACE_VERSION013"), Some((Kind::UserStats, 13)));
        assert_eq!(classify("SteamInput006"), Some((Kind::Input, 6)));
        assert_eq!(classify("SteamNetworkingUtils004"), Some((Kind::Stub, 4)));
        assert_eq!(classify("SteamMatchMaking009"), Some((Kind::Stub, 9)));
        assert_eq!(classify("STEAMREMOTESTORAGE_INTERFACE_VERSION016"), Some((Kind::Stub, 16)));
        assert_eq!(classify("STEAMAPPTICKET_INTERFACE_VERSION001"), Some((Kind::Stub, 1)));
        assert_eq!(classify("SteamClient"), None);
        assert_eq!(classify("SteamClient02x"), None);
        assert_eq!(classify("garbage"), None);
        assert_eq!(classify(""), None);
    }

    #[test]
    fn every_version_in_the_ranges_has_a_table() {
        for (prefix, kind, range) in IMPLEMENTED {
            for n in range {
                let ver = format!("{prefix}{n:03}");
                assert_eq!(classify(&ver), Some((kind, n)), "{ver}");
                if kind != Kind::Input {
                    assert!(slots(kind, n).len() > 20, "{ver}");
                }
            }
        }
    }
}
