#![allow(non_snake_case)]
use colored::ColoredString;
use colored::Colorize;
use http::Response;
use http::StatusCode;
use isahc::prelude::*;
use isahc::Body;
use isahc::Request;
use progress_report::MediaSourceInfo;
use regex::Regex;
use serde_derive::Deserialize;
use std::io;
use std::io::prelude::*;
use std::io::stdin;
use std::process;
use std::process::Child;
use std::process::Command;
use std::process::ExitCode;
use std::thread;
use std::time::Duration;
use sysinfo::System;
use urlencoding::encode;
pub mod config;
pub mod discord;
pub mod mediaserver_information;
pub mod player;
mod progress_report;
pub mod settings;
use mediaserver_information::*;
use player::play;
use settings::*;
const APPNAME: &str = "Puddler";
const VERSION: &str = env!("CARGO_PKG_VERSION");
use app_dirs::AppInfo;
const APP_INFO: AppInfo = AppInfo {
    name: APPNAME,
    author: "VernoxVernax",
};

#[derive(Debug, Deserialize)]
struct ItemJson {
    Items: Vec<Items>,
    TotalRecordCount: Option<u16>,
}

#[derive(Debug, Deserialize, PartialEq, Clone)]
pub struct Items {
    pub Name: String,
    pub Id: String,
    pub RunTimeTicks: Option<u64>,
    pub Type: String,
    pub UserData: UserData,
    pub SeriesName: Option<String>,
    pub SeriesId: Option<String>,
    pub SeasonName: Option<String>,
    pub SeasonId: Option<String>,
    pub PremiereDate: Option<String>,
    pub MediaSources: Option<Vec<MediaSourceInfo>>,
}

#[derive(Debug, Deserialize, PartialEq, Clone)]
pub struct UserData {
    pub PlayedPercentage: Option<f64>,
    pub PlaybackPositionTicks: i64,
    pub Played: bool,
}

#[derive(Debug, Deserialize, PartialEq, Clone)]
struct SeriesStruct {
    Items: Vec<Seasons>,
}

#[derive(Debug, Deserialize, PartialEq, Clone)]
struct Seasons {
    Name: String,
    Id: String,
    Type: String,
    UserData: UserData,
    SeriesName: String,
    SeriesId: String,
}

#[derive(Debug, Deserialize, PartialEq, Clone)]
struct SeasonStruct {
    Items: Vec<Items>,
}

fn main() -> ExitCode {
    let mut settings: Settings = initialize_settings(0);
    println!(
        "{}",
        r"     ____            __    ____         
    / __ \__  ______/ /___/ / /__  _____
   / /_/ / / / / __  / __  / / _ \/ ___/
  / ____/ /_/ / /_/ / /_/ / /  __/ /    
 /_/    \__,_/\__,_/\__,_/_/\___/_/"
            .to_string()
            .bright_cyan()
    );
    println!();
    loop {
        if settings.server_config.is_some() {
            print!("  [ENTER] Stream from default media-server\n  [1] Stream from either Emby or Jellyfin\n  [2] Change puddlers default settings\n  [3] Display current settings\n  [E] Exit puddler");
            let menu = getch("123Ee\n");
            match menu {
                '\n' => break,
                '1' => {
                    settings.server_config = None;
                    break;
                }
                '2' => {
                    settings = initialize_settings(1);
                }
                '3' => {
                    settings = initialize_settings(2);
                }
                'e' | 'E' => {
                    process::exit(0x0100);
                }
                _ => (),
            };
        } else {
            print!("  [1] Stream from either Emby or Jellyfin\n  [2] Change puddlers default settings\n  [3] Display current settings\n  [E] Exit puddler");
            let menu = getch("123Ee");
            match menu {
                '1' => break,
                '2' => {
                    settings = initialize_settings(1);
                }
                '3' => {
                    settings = initialize_settings(2);
                }
                'e' | 'E' => {
                    process::exit(0x0100);
                }
                _ => (),
            };
        }
    }
    if let Some(head_dict) = check_information(&settings) {
        loop {
            choose_and_play(&head_dict, &settings);
        }
    } else {
        ExitCode::FAILURE
    }
}

fn choose_and_play(head_dict: &HeadDict, settings: &Settings) {
    let ipaddress = &head_dict.config_file.ipaddress;
    let media_server = &head_dict.media_server;
    let user_id = &head_dict.config_file.user_id;

    // nextup & resume
    let mut item_list: Vec<Items> = Vec::new();
    let pick: Option<i32>;
    let nextup = puddler_get(
        format!(
            "{}{}/Users/{}/Items/Resume?Fields=PremiereDate,MediaSources",
            &ipaddress, &media_server, &user_id
        ),
        head_dict,
    );
    let response: ItemJson = match nextup {
        Ok(mut t) => {
            let response_text = &t.text().unwrap();
            serde_json::from_str(response_text).unwrap()
        }
        Err(e) => {
            println!(
                "Your network connection seems to be limited. Error: {e}\nUnable to continue."
            );
            process::exit(0x0100);
        }
    };
    if response.TotalRecordCount.unwrap() != 0 {
        println!("\nContinue Watching:");
        item_list = print_menu(&response, true, item_list);
    }
    if media_server != "/emby" {
        let jellyfin_nextup = puddler_get(
            format!(
                "{}{}/Shows/NextUp?Fields=PremiereDate,MediaSources&UserId={}",
                &ipaddress, &media_server, &user_id
            ),
            head_dict,
        );
        let jellyfin_response: ItemJson = match jellyfin_nextup {
            Ok(mut t) => {
                let jellyfin_response_text = &t.text().unwrap();
                serde_json::from_str(jellyfin_response_text).unwrap()
            }
            Err(e) => panic!("failed to parse get request: {e}"),
        };
        if jellyfin_response.TotalRecordCount.unwrap() != 0 {
            if response.TotalRecordCount.unwrap() == 0 {
                println!("\nContinue Watching:");
            }
            item_list = print_menu(&jellyfin_response, true, item_list);
        }
    }

    // latest
    let latest_series = puddler_get(format!("{}{}/Users/{}/Items/Latest?Limit=10&IncludeItemTypes=Episode&Fields=PremiereDate,MediaSources", &ipaddress, &media_server, &user_id), head_dict);
    let latest_series_response: ItemJson = match latest_series {
        Ok(mut t) => {
            let response_text = format!("{{\"Items\":{}}}", t.text().unwrap());
            serde_json::from_str(&response_text).unwrap()
        }
        Err(e) => panic!("failed to parse get request: {e}"),
    };
    if !latest_series_response.Items.is_empty() {
        println!("\nLatest:");
        item_list = print_menu(&latest_series_response, true, item_list);
    }
    let latest = puddler_get(format!("{}{}/Users/{}/Items/Latest?Limit=10&IncludeItemTypes=Movie&Fields=PremiereDate,MediaSources", &ipaddress, &media_server, &user_id), head_dict);
    let latest_response: ItemJson = match latest {
        Ok(mut t) => {
            let response_text = format!("{{\"Items\":{}}}", t.text().unwrap());
            serde_json::from_str(&response_text).unwrap()
        }
        Err(e) => panic!("failed to parse get request: {e}"),
    };
    if !latest_response.Items.is_empty() {
        if latest_series_response.Items.is_empty() {
            println!("\nLatest:");
        }
        item_list = print_menu(&latest_response, true, item_list);
    }
    print!("Please choose from above, enter a search term, or type \"ALL\" to display literally everything.\n: ");
    io::stdout().flush().expect("Failed to flush stdout");
    let mut input = String::new();
    io::stdin().read_line(&mut input).unwrap();

    // processing input
    if input.trim() == "ALL" {
        let all = puddler_get(format!("{}{}/Items?UserId={}&Recursive=true&IncludeItemTypes=Series,Movie&Fields=PremiereDate,MediaSources&collapseBoxSetItems=False", &ipaddress, &media_server, &user_id), head_dict);
        let all_response: ItemJson = match all {
            Ok(mut t) => {
                let response_text: &String = &t.text().unwrap();
                serde_json::from_str(response_text).unwrap()
            }
            Err(e) => panic!("failed to parse get request: {e}"),
        };
        item_list = Vec::new();
        item_list = print_menu(&all_response, false, item_list);

        if all_response.Items.len() > 1 {
            print!(": ");
            io::stdout().flush().expect("Failed to flush stdout");
        }
        pick = process_input(&item_list, None);
    } else if is_numeric(&input) {
        pick = process_input(&item_list, Some(input.trim().to_string()));
    } else {
        input = encode(input.trim()).into_owned();
        let search = puddler_get(format!("{}{}/Items?SearchTerm={}&UserId={}&Recursive=true&IncludeItemTypes=Series,Movie&Fields=PremiereDate,MediaSources&collapseBoxSetItems=False", &ipaddress, &media_server, &input, &user_id), head_dict);
        let search_response: ItemJson = match search {
            Ok(mut t) => {
                let search_text: &String = &t.text().unwrap();
                serde_json::from_str(search_text).unwrap()
            }
            Err(e) => panic!("failed to parse get request: {e}"),
        };

        if !search_response.Items.is_empty() {
            item_list = Vec::new();
            item_list = print_menu(&search_response, false, item_list);
            if search_response.Items.len() > 1 {
                print!(": ");
                io::stdout().flush().expect("Failed to flush stdout");
            }
            pick = process_input(&item_list, None);
        } else {
            println!("\nNo results found for: {}.", input.to_string().bold());
            pick = None
        }
    }

    if let Some(pick) = pick {
        item_parse(head_dict, &item_list, pick, settings);
    }
}

fn puddler_get(url: String, head_dict: &HeadDict) -> Result<Response<Body>, isahc::Error> {
    let request_header = &head_dict.request_header;
    let response: Response<Body> = Request::get(url)
        .timeout(Duration::from_secs(5))
        .header("X-Application", &request_header.application)
        .header("X-Emby-Token", &request_header.token)
        .header("Content-Type", "application/json")
        .body(())?
        .send()?;
    let result = match response.status() {
        StatusCode::OK => response,
        _ => panic!(
            "{} your server is missing some api endpoints, i think",
            response.status()
        ),
    };
    Ok(result)
}

fn is_numeric(input: &str) -> bool {
    for x in input.trim().chars() {
        if x.is_alphabetic() {
            return false;
        }
    }
    true
}

fn process_input(item_list: &Vec<Items>, number: Option<String>) -> Option<i32> {
    let items_in_list = item_list.len().try_into().unwrap();
    match items_in_list {
        n if n > 1 => {
            let mut raw_input: String;
            if let Some(res) = number.as_ref() {
                raw_input = res.to_string();
            } else {
                raw_input = String::new();
                io::stdin().read_line(&mut raw_input).unwrap();
                raw_input = raw_input.trim().to_string();
            }
            let pick = raw_input.parse::<i32>().unwrap();
            if pick < items_in_list + 1 && pick >= 0 {
                let item = item_list.get(pick as usize).unwrap();
                if item.SeasonName == Some("Specials".to_string()) {
                    let first_occurence = item_list.iter().position(|i| i.Id == item.Id);
                    let first = first_occurence == Some(pick as usize);
                    let embedded: ColoredString = if number.is_some() || !first {
                        "Embedded".bold()
                    } else {
                        "Embedded".strikethrough()
                    };
                    println!(
                        "\nYou've chosen {}. ({})\n",
                        format!(
                            "{} ({}) - {} - {}",
                            item.SeriesName.as_ref().unwrap(),
                            (&item.PremiereDate.as_ref().unwrap_or(&"????".to_string())[0..4]),
                            item.SeasonName.as_ref().unwrap(),
                            item.Name
                        )
                        .cyan(),
                        embedded
                    );
                } else if item.Type == "Episode" {
                    println!(
                        "\nYou've chosen {}.\n",
                        format!(
                            "{} ({}) - {} - {}",
                            item.SeriesName.as_ref().unwrap(),
                            (&item.PremiereDate.as_ref().unwrap_or(&"????".to_string())[0..4]),
                            item.SeasonName.as_ref().unwrap(),
                            item.Name
                        )
                        .cyan()
                    );
                } else {
                    println!(
                        "\nYou've chosen {}.\n",
                        format!(
                            "{} ({})",
                            item.Name,
                            &item.PremiereDate.as_ref().unwrap_or(&"????".to_string())[0..4]
                        )
                        .cyan()
                    );
                }
            } else {
                println!("{}", "Are you ok?!".red());
                process::exit(0x0100);
            }
            Some(pick)
        }
        1 => {
            let mut raw_input = String::new();
            io::stdin().read_line(&mut raw_input).unwrap();
            let pick: i32 = 0;
            Some(pick)
        }
        _ => None,
    }
}

fn item_parse(head_dict: &HeadDict, item_list: &[Items], pick: i32, settings: &Settings) {
    let ipaddress: &String = &head_dict.config_file.ipaddress;
    let media_server: &String = &head_dict.media_server;
    let user_id: &String = &head_dict.config_file.user_id;

    if item_list.get(pick as usize).unwrap().Type == *"Movie" {
        let item = item_list.get(pick as usize).unwrap();
        play(settings, head_dict, item);
    } else if item_list.get(pick as usize).unwrap().Type == *"Series" {
        let series = &item_list.get(pick as usize).unwrap();
        println!("{}:", series.Name);
        let series_response = puddler_get(format!("{}{}/Users/{}/Items?ParentId={}&Fields=PremiereDate,MediaSources&collapseBoxSetItems=False", &ipaddress, &media_server, &user_id, &series.Id), head_dict);
        let series_json: SeriesStruct = match series_response {
            Ok(mut t) => {
                let parse_text: &String = &t.text().unwrap();
                serde_json::from_str(parse_text).unwrap()
            }
            Err(e) => panic!("failed to parse series request: {e}"),
        };
        println!("{:?}", &series_json);

        let item_list: Vec<Items> = process_series(&series_json, head_dict, true);
        let items_in_list: i32 = item_list.len().try_into().unwrap();

        let filtered_input: i32 = if items_in_list > 1 {
            print!("Please enter which episode you want to continue at.\n: ");
            io::stdout().flush().expect("Failed to flush stdout");
            process_input(&item_list, None).unwrap()
        } else {
            0
        };

        series_play(&item_list, filtered_input, head_dict, settings);
    } else if "Episode"
        .to_string()
        .contains(&item_list.get(pick as usize).unwrap().Type)
    {
        let item: &Items = item_list.get(pick as usize).unwrap();
        let series_response = puddler_get(format!("{}{}/Users/{}/Items?ParentId={}&Fields=PremiereDate,MediaSources&collapseBoxSetItems=False", &ipaddress, &media_server, &user_id, &item.SeriesId.as_ref().unwrap()), head_dict);
        let series_json: SeriesStruct = match series_response {
            Ok(mut t) => {
                let parse_text: &String = &t.text().unwrap();
                serde_json::from_str(parse_text).unwrap()
            }
            Err(e) => panic!("failed to parse series request: {e}"),
        };
        println!("{:?}", &series_json);
        let item_list: Vec<Items> = process_series(&series_json, head_dict, false);
        let mut item_pos: i32 = 0;
        let mut amount = item_list.iter().filter(|&i| i.Id == item.Id).count(); // how many times the episode exists in the list
        if item.SeasonName == Some("Specials".to_string()) && amount > 1 {
            for (things, item1) in item_list.iter().enumerate() {
                if item1.Id == item.Id {
                    if amount == 1 {
                        item_pos = things.try_into().unwrap();
                        break;
                    } else {
                        amount -= 1;
                    }
                }
            }
        } else {
            for (things, item1) in item_list.iter().enumerate() {
                if item1.Id == item.Id {
                    item_pos = things.try_into().unwrap();
                    break;
                }
            }
        }
        series_play(&item_list, item_pos, head_dict, settings);
    }
}

fn mpv播放(播放开始时间_秒: &i32, 内容: &str, 标题: &str, 字幕: &str) -> Child {
    let mpv = Command::new(r"E:\video\mpv_config-latest\mpv.exe")
        .args(&[
            内容,
            (format!("--start={}", 播放开始时间_秒)).as_str(),
            "--fs",
            "--idle=once",
            "--pause",
            (format!("--force-media-title={}", 标题)).as_str(),
            (format!("--sub-file={}", 字幕)).as_str(),
            // ={sub_file}
        ])
        .spawn();
    mpv.unwrap()
}
fn 获取整数输入() -> i32 {
    println!("请输入整数");
    let mut input = String::new();
    stdin()
        .read_line(&mut input)
        .ok()
        .expect("Failed to read line");
    let input = input.trim().parse::<i32>().unwrap();
    input
}

fn 提取整数(输入: &str) -> i32 {
    let re = Regex::new(r"(?<整数>\d+)").unwrap();
    let Some(caps) = re.captures(输入) else {
        println!("没匹配到季数 {}", 输入);
        return 66;
    };
    caps["整数"].parse::<i32>().unwrap()
}
use glob::glob;

fn 寻找匹配的字幕(季: i32, 集: &String) -> String {
    let 字幕文件夹 = r"R:\sub_out\";
    let 需要的字幕 = format!("*S*{}E{}*.ass", 季, 集);
    let 需要的字幕匹配表达式 = format!("{}{}", 字幕文件夹, 需要的字幕);
    let 匹配的字幕列表 = glob(需要的字幕匹配表达式.as_str()).unwrap();
    let mut 字幕列表: Vec<std::path::PathBuf> = Vec::new();
    for entry in 匹配的字幕列表 {
        // println!("{}", entry.unwrap().display());
        字幕列表.push(entry.unwrap());
    }
    if 字幕列表.len() == 1 {
        字幕列表[0].to_str().unwrap().to_string()
    } else {
        "None".to_string()
    }
    // println!("{}", 匹配的字幕列表[0]);
}

fn 获取程序数量(目标数量: i32) -> bool {
    let mut s = System::new_all();
    loop {
        std::thread::sleep(Duration::from_secs(30));
        s.refresh_processes();
        let 程序查找 = s.processes_by_name("mpv.exe");
        let mut 数量 = 0;
        for _process in 程序查找 {
            数量 += 1
        }
        if 数量 < 目标数量.try_into().unwrap() {
            break;
        }
    }
    return true;
}

fn series_play(item_list: &Vec<Items>, mut pick: i32, head_dict: &HeadDict, settings: &Settings) {
    let episode_amount: i32 = item_list.len().try_into().unwrap();
    // TODO 这个集数判断输出是900+ 是总集数(所有季) 但确实在pick +2 的判断 判断出了季的分割点 此时pick应在190左右
    // TODO 可能是某种打开方式只判断了单季 无论是搜索还是 等待观看进入都没复现
    // let item = &item_list.get(pick as usize).unwrap();
    //TODO 集数偏移 用于匹配字幕 实测偏移了 1 就不用 确定季数了 这里显示的6 字幕用的7 哦 index从0开始的 默认给偏移为1
    // TODO mpv播放器始终3个 然后把字幕给实现了就行了
    let mut 自定义播放开始时间_秒 = 140;
    let mut 初始化多进程播放 = 0;
    let mut 多进程播放状态 = false;
    let mut 字幕偏移集数 = 1;
    // let 播放地址 = format!("{}{}/Videos/{}/stream?Container=mkv&Static=true&api_key={}",head_dict.config_file.ipaddress, head_dict.media_server, item.Id, head_dict.request_header.token);
    // let 当前播放 = mpv播放(&播放地址,&标题);
    let 一直往后缓存的集数 = 2;
    let 总进程数 = 一直往后缓存的集数 + 1;

    // for _x in 1..=一直往后缓存的集数{
    //   pick += 1;
    //   if item_list.get(pick as usize).is_some() {
    //     let item = &item_list.get(pick as usize).unwrap();
    //     let 播放地址 = format!("{}{}/Videos/{}/stream?Container=mkv&Static=true&api_key={}",head_dict.config_file.ipaddress, head_dict.media_server, item.Id, head_dict.request_header.token);
    //     mpv播放(&播放地址,&标题);}
    // }
    // play(settings, head_dict, item);
    loop {
        if (pick + 1) > episode_amount {
            // +1 since episode_amount doesn't start at 0 AND +1 for next ep 多次+2却忽略了最后一集改为1
            println!("{} {}", pick, episode_amount);

            println!("\nYou've reached the end of your episode list. Returning to menu ...");
            break;
        } else {
            if item_list.get(pick as usize).is_some() {
                let next_item = &item_list.get(pick as usize).unwrap();
                // if next_item.UserData.Played {
                //     continue;
                // };
                let 当前季数 = 提取整数(next_item.SeasonName.as_ref().unwrap());
                let 当前集数 = format!("{:0>2}", pick + 字幕偏移集数);
                let 标题 = format!(
                    "{} - {} S{}E{}",
                    next_item.Name,
                    next_item.SeriesName.as_ref().unwrap(),
                    &当前季数,
                    &当前集数
                )
                .cyan();
                let 当前字幕 = 寻找匹配的字幕(当前季数, &当前集数);
                let 播放地址 = format!(
                    "{}{}/Videos/{}/stream?Container=mkv&Static=true&api_key={}",
                    head_dict.config_file.ipaddress,
                    head_dict.media_server,
                    next_item.Id,
                    head_dict.request_header.token
                );
                // println!("{:?}", &next_item);
                if 多进程播放状态 && 初始化多进程播放 <= 一直往后缓存的集数
                {
                    pick += 1;
                    mpv播放(&自定义播放开始时间_秒, &播放地址, &标题, &当前字幕);
                    初始化多进程播放 += 1;
                    println!("初始化多进程播放 {}", &标题);
                    thread::sleep(Duration::from_secs(20));
                    // 等待以确保窗口按剧集顺序排列
                    continue;
                };
                if 初始化多进程播放 == 总进程数 {
                    // 初始化多进程完毕后运行的语句
                    let 需放下一集了 = 获取程序数量(总进程数);
                    if 需放下一集了 {
                        pick += 1;
                        mpv播放(&自定义播放开始时间_秒, &播放地址, &标题, &当前字幕);
                        println!("缓冲 {}", &标题);

                        continue;
                    }
                }
                if settings.autoplay {
                    println!("\nWelcome back. Continuing in 3 seconds:\n{}", &标题);

                    thread::sleep(Duration::from_secs(3));
                    pick += 1;
                    mpv播放(&自定义播放开始时间_秒, &播放地址, &标题, &当前字幕);
                } else {
                    println!(
                        "\nWelcome back. Do you want to continue playback with:\n{}",
                        标题
                    );
                    print!(" (N)ext | (M)enu | (E)xit");
                    let cont = getch("NnDdSsFfCcAaPpEeMm");
                    match cont {
                        'N' | 'n' => {
                            pick += 1;
                            mpv播放(&自定义播放开始时间_秒, &播放地址, &标题, &当前字幕);
                        }
                        'D' | 'd' => {
                            多进程播放状态 = true;
                            初始化多进程播放 = 0;
                            // 即使没有播放每次循环都把pick相加了
                        }
                        'S' | 's' => {
                            println!(
                                "总集数 {} 当前集数 {} 请输入集数 输入值会与偏移相加",
                                &episode_amount, &当前集数
                            );
                            pick = 获取整数输入();
                            pick -= 字幕偏移集数;
                        }
                        'F' | 'f' => {
                            println!("当前开始播放秒 {} eg:140", &自定义播放开始时间_秒);
                            自定义播放开始时间_秒 = 获取整数输入();
                        }
                        'C' | 'c' => {
                            字幕偏移集数 = 获取整数输入();
                        }
                        'A' | 'a' => {
                            println!("{}", &episode_amount)
                        }
                        'P' | 'p' => {
                            // let item = &item_list.get(pick as usize).unwrap();
                            // println!("{:?}",&item_list)
                            for i in item_list {
                                println!("名 {:?} id {:?}", i, i.Id);
                                // println!("名 {:?} id {:?} 播放id {:?}",i.Name,i.Id,i.MediaSources.as_ref().unwrap()[0].Id);
                                let 播放地址 = format!(
                                    "{}{}/Videos/{}/stream?Container=mkv&Static=true&api_key={}",
                                    head_dict.config_file.ipaddress,
                                    head_dict.media_server,
                                    i.Id,
                                    head_dict.request_header.token
                                );
                                println!("{}", &播放地址);
                            }
                        }
                        'M' | 'm' => break,
                        'E' | 'e' => {
                            process::exit(0x0100);
                        }
                        _ => (),
                    }
                }
            } else {
                break;
            }
        }
    }
}

fn process_series(series: &SeriesStruct, head_dict: &HeadDict, printing: bool) -> Vec<Items> {
    let ipaddress: &String = &head_dict.config_file.ipaddress;
    let media_server: &String = &head_dict.media_server;
    let user_id: &String = &head_dict.config_file.user_id;
    let mut index_iterator: i32 = 0;
    let mut episode_list: Vec<Items> = Vec::new();

    for season_numb in 0..series.Items.len() {
        let last_season = series.Items.len() == season_numb + 1;
        let season_branches = if last_season { "└─" } else { "├─" };
        let season: Seasons = series.Items[season_numb].clone();
        if printing {
            println!("  {} {}", season_branches, season.Name);
        }
        let season_res = puddler_get(format!("{}{}/Users/{}/Items?ParentId={}&Fields=PremiereDate,MediaSources&collapseBoxSetItems=False", &ipaddress, &media_server, &user_id, &season.Id), head_dict);
        let season_json: SeasonStruct = match season_res {
            Ok(mut t) => {
                let parse_text: &String = &t.text().unwrap().to_string();
                serde_json::from_str(parse_text).unwrap()
            }
            Err(e) => panic!("failed to parse series request: {e}"),
        };
        for episode_numb in 0..season_json.Items.len() {
            // for the code readers: the "season_json" vector is obviously different to "season" since the latter doesn't include any episodes.
            let episode: Items = season_json.Items[episode_numb].clone();
            let last_episode = season_json.Items.len() == episode_numb + 1;
            let episode_branches = if last_episode && last_season {
                "     └──"
            } else if last_episode && !last_season {
                "│    └──"
            } else if !last_episode && last_season {
                "     ├──"
            } else {
                "│    ├──"
            };
            if !episode_list.contains(&episode)
                || episode.SeasonName == Some("Specials".to_string())
            {
                episode_list.push(season_json.Items[episode_numb].clone());
            }
            if !printing {
                continue;
            };
            let extra = if episode.SeasonName != Some(season.Name.clone()) {
                // If the special is listed in a normal season, the season name of it is different from the actual season which the special is assigned to (kinda makes sense to avoid duplicate items)
                " (S)".to_string()
            } else {
                "".to_string()
            };
            if episode.UserData.PlayedPercentage.is_some() {
                let long_perc: f64 = episode.UserData.PlayedPercentage.unwrap();
                println!(
                    "  {} [{}] {}{} {}% ",
                    episode_branches,
                    index_iterator,
                    episode.Name,
                    extra,
                    long_perc.round() as i64
                )
            } else if episode.UserData.Played {
                println!(
                    "  {} [{}] {}{} {} ",
                    episode_branches,
                    index_iterator,
                    episode.Name,
                    extra,
                    "[PLAYED]".to_string().green()
                );
            } else {
                println!(
                    "  {} [{}] {}{}",
                    episode_branches, index_iterator, episode.Name, extra
                );
            };
            index_iterator += 1;
        }
    }
    episode_list
}

fn print_menu(items: &ItemJson, recommendation: bool, mut item_list: Vec<Items>) -> Vec<Items> {
    let count: usize = if recommendation { 2 } else { items.Items.len() };
    if count > 1 && !recommendation {
        println!("\nPlease choose from the following results:")
    }
    for h in 0..items.Items.len() {
        let x: Items = items.Items[h].clone();
        if !item_list.contains(&x) {
            item_list.push(items.Items[h].clone());
            if !x.UserData.Played {
                if x.UserData.PlayedPercentage.is_some() {
                    let long_perc: f64 = x.UserData.PlayedPercentage.unwrap();
                    let percentage = format!("{}%", long_perc.round() as i64); // Pardon the `.round`
                    if count != 1 {
                        if x.Type == *"Episode" || x.Type == *"Special" {
                            println!(
                                "      [{}] {} ({}) - {} - {} - ({}) {}",
                                &item_list.iter().position(|y| y == &x).unwrap(),
                                x.SeriesName.unwrap(),
                                &x.PremiereDate.unwrap_or("????".to_string())[0..4],
                                x.SeasonName.unwrap(),
                                x.Name,
                                x.Type,
                                percentage
                            );
                        } else {
                            println!(
                                "      [{}] {} ({}) - ({}) {}",
                                &item_list.iter().position(|y| y == &x).unwrap(),
                                x.Name,
                                &x.PremiereDate.unwrap_or("????".to_string())[0..4],
                                x.Type,
                                percentage
                            );
                        }
                    } else {
                        println!("\nOnly one item has been found.\nDo you want to select this title?\n      {}", format!("[Enter] {} ({}) - ({})", x.Name, &x.PremiereDate.unwrap_or("????".to_string())[0..4], x.Type).cyan());
                    }
                } else if count != 1 {
                    if x.Type == *"Episode" || x.Type == *"Special" {
                        println!(
                            "      [{}] {} ({}) - {} - {} - ({})",
                            &item_list.iter().position(|y| y == &x).unwrap(),
                            x.SeriesName.unwrap(),
                            &x.PremiereDate.unwrap_or("????".to_string())[0..4],
                            x.SeasonName.unwrap(),
                            x.Name,
                            x.Type
                        );
                    } else {
                        println!(
                            "      [{}] {} ({}) - ({})",
                            &item_list.iter().position(|y| y == &x).unwrap(),
                            x.Name,
                            &x.PremiereDate.unwrap_or("????".to_string())[0..4],
                            x.Type
                        );
                    }
                } else {
                    println!("\nOnly one item has been found.\nDo you want to select this title?\n      {}", format!("[Enter] {} ({}) - ({})", x.Name, &x.PremiereDate.unwrap_or("????".to_string())[0..4], x.Type).cyan());
                }
            } else if count != 1 {
                if x.Type == *"Episode" || x.Type == *"Special" {
                    println!(
                        "      [{}] {} ({}) - {} - {} - ({})  {}",
                        &item_list.iter().position(|y| y == &x).unwrap(),
                        x.SeriesName.unwrap(),
                        &x.PremiereDate.unwrap_or("????".to_string())[0..4],
                        x.SeasonName.unwrap(),
                        x.Name,
                        x.Type,
                        "[PLAYED]".to_string().green()
                    );
                } else {
                    println!(
                        "      [{}] {} ({}) - ({})  {}",
                        &item_list.iter().position(|y| y == &x).unwrap(),
                        x.Name,
                        &x.PremiereDate.unwrap_or("????".to_string())[0..4],
                        x.Type,
                        "[PLAYED]".to_string().green()
                    );
                }
            } else {
                println!(
                    "\nOnly one item has been found.\nDo you want to select this title?\n      {}",
                    format!(
                        "[Enter] {} ({}) - ({})",
                        x.Name,
                        &x.PremiereDate.unwrap_or("????".to_string())[0..4],
                        x.Type
                    )
                    .cyan()
                );
            }
        }
    }
    item_list
}
