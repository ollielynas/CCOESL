//! Every Phosphor icon by name, for `:name:` shortcodes in documents.
//!
//! Generated from `egui_phosphor::regular` 0.14 (the font the shell loads). Names are
//! Phosphor's own in `snake_case`, as in `:folder_open:`. To regenerate, list every
//! `pub const NAME: &str = "\u{XXXX}";` in that crate's `src/variants/codepoints.rs`,
//! lowercase the names, sort them, and write them and their codepoints in that order.
//!
//! Stored as one string of names and a parallel array of codepoints rather than a table of
//! `(&str, char)`: a pointer and a length per entry would cost this module an extra ~17 KB
//! gzipped, paid by every client that downloads it.

/// Every icon name, sorted, one per line.
pub const NAMES: &str = "\
acorn\nactivity\naddress_book\naddress_book_tabs\nair_traffic_control\nairplane\n\
airplane_in_flight\nairplane_landing\nairplane_takeoff\nairplane_taxiing\nairplane_tilt\n\
airplay\nalarm\nalien\nalign_bottom\nalign_bottom_simple\nalign_center_horizontal\n\
align_center_horizontal_simple\nalign_center_vertical\nalign_center_vertical_simple\n\
align_left\nalign_left_simple\nalign_right\nalign_right_simple\nalign_top\n\
align_top_simple\namazon_logo\nambulance\nanchor\nanchor_simple\nandroid_logo\nangle\n\
angular_logo\naperture\napp_store_logo\napp_window\napple_logo\napple_podcasts_logo\n\
approximate_equals\narchive\narchive_box\narchive_tray\narmchair\narrow_arc_left\n\
arrow_arc_right\narrow_bend_double_up_left\narrow_bend_double_up_right\n\
arrow_bend_down_left\narrow_bend_down_right\narrow_bend_left_down\narrow_bend_left_up\n\
arrow_bend_right_down\narrow_bend_right_up\narrow_bend_up_left\narrow_bend_up_right\n\
arrow_circle_down\narrow_circle_down_left\narrow_circle_down_right\narrow_circle_left\n\
arrow_circle_right\narrow_circle_up\narrow_circle_up_left\narrow_circle_up_right\n\
arrow_clockwise\narrow_counter_clockwise\narrow_down\narrow_down_left\narrow_down_right\n\
arrow_elbow_down_left\narrow_elbow_down_right\narrow_elbow_left\narrow_elbow_left_down\n\
arrow_elbow_left_up\narrow_elbow_right\narrow_elbow_right_down\narrow_elbow_right_up\n\
arrow_elbow_up_left\narrow_elbow_up_right\narrow_fat_down\narrow_fat_left\n\
arrow_fat_line_down\narrow_fat_line_left\narrow_fat_line_right\narrow_fat_line_up\n\
arrow_fat_lines_down\narrow_fat_lines_left\narrow_fat_lines_right\narrow_fat_lines_up\n\
arrow_fat_right\narrow_fat_up\narrow_left\narrow_line_down\narrow_line_down_left\n\
arrow_line_down_right\narrow_line_left\narrow_line_right\narrow_line_up\n\
arrow_line_up_left\narrow_line_up_right\narrow_right\narrow_square_down\n\
arrow_square_down_left\narrow_square_down_right\narrow_square_in\narrow_square_left\n\
arrow_square_out\narrow_square_right\narrow_square_up\narrow_square_up_left\n\
arrow_square_up_right\narrow_u_down_left\narrow_u_down_right\narrow_u_left_down\n\
arrow_u_left_up\narrow_u_right_down\narrow_u_right_up\narrow_u_up_left\narrow_u_up_right\n\
arrow_up\narrow_up_left\narrow_up_right\narrows_clockwise\narrows_counter_clockwise\n\
arrows_down_up\narrows_horizontal\narrows_in\narrows_in_cardinal\n\
arrows_in_line_horizontal\narrows_in_line_vertical\narrows_in_simple\narrows_left_right\n\
arrows_merge\narrows_out\narrows_out_cardinal\narrows_out_line_horizontal\n\
arrows_out_line_vertical\narrows_out_simple\narrows_split\narrows_vertical\narticle\n\
article_medium\narticle_ny_times\nasclepius\nasterisk\nasterisk_simple\nat\natom\n\
avocado\naxe\nbaby\nbaby_carriage\nbackpack\nbackspace\nbag\nbag_simple\nballoon\n\
bandaids\nbank\nbarbell\nbarcode\nbarn\nbarricade\nbaseball\nbaseball_cap\n\
baseball_helmet\nbasket\nbasketball\nbathtub\nbattery_charging\n\
battery_charging_vertical\nbattery_empty\nbattery_full\nbattery_high\nbattery_low\n\
battery_medium\nbattery_plus\nbattery_plus_vertical\nbattery_vertical_empty\n\
battery_vertical_full\nbattery_vertical_high\nbattery_vertical_low\n\
battery_vertical_medium\nbattery_warning\nbattery_warning_vertical\nbeach_ball\nbeanie\n\
bed\nbeer_bottle\nbeer_stein\nbehance_logo\nbell\nbell_ringing\nbell_simple\n\
bell_simple_ringing\nbell_simple_slash\nbell_simple_z\nbell_slash\nbell_z\nbelt\n\
bezier_curve\nbicycle\nbinary\nbinoculars\nbiohazard\nbird\nblueprint\nbluetooth\n\
bluetooth_connected\nbluetooth_slash\nbluetooth_x\nboat\nbomb\nbone\nbook\nbook_bookmark\n\
book_open\nbook_open_text\nbook_open_user\nbookmark\nbookmark_simple\nbookmarks\n\
bookmarks_simple\nbooks\nboot\nboules\nbounding_box\nbowl_food\nbowl_steam\nbowling_ball\n\
box_arrow_down\nbox_arrow_up\nboxing_glove\nbrackets_angle\nbrackets_curly\n\
brackets_round\nbrackets_square\nbrain\nbrandy\nbread\nbridge\nbriefcase\n\
briefcase_metal\nbroadcast\nbroom\nbrowser\nbrowsers\nbug\nbug_beetle\nbug_droid\n\
building\nbuilding_apartment\nbuilding_office\nbuildings\nbulldozer\nbus\nbutterfly\n\
cable_car\ncactus\ncaduceus\ncake\ncalculator\ncalendar\ncalendar_blank\ncalendar_check\n\
calendar_dot\ncalendar_dots\ncalendar_heart\ncalendar_minus\ncalendar_plus\n\
calendar_slash\ncalendar_star\ncalendar_x\ncall_bell\ncamera\ncamera_plus\ncamera_rotate\n\
camera_slash\ncampfire\ncar\ncar_battery\ncar_profile\ncar_simple\ncardholder\ncards\n\
cards_three\ncaret_circle_double_down\ncaret_circle_double_left\n\
caret_circle_double_right\ncaret_circle_double_up\ncaret_circle_down\ncaret_circle_left\n\
caret_circle_right\ncaret_circle_up\ncaret_circle_up_down\ncaret_double_down\n\
caret_double_left\ncaret_double_right\ncaret_double_up\ncaret_down\ncaret_left\n\
caret_line_down\ncaret_line_left\ncaret_line_right\ncaret_line_up\ncaret_right\ncaret_up\n\
caret_up_down\ncarrot\ncash_register\ncassette_tape\ncastle_turret\ncat\n\
cell_signal_full\ncell_signal_high\ncell_signal_low\ncell_signal_medium\n\
cell_signal_none\ncell_signal_slash\ncell_signal_x\ncell_tower\ncertificate\nchair\n\
chalkboard\nchalkboard_simple\nchalkboard_teacher\nchampagne\ncharging_station\n\
chart_bar\nchart_bar_horizontal\nchart_donut\nchart_line\nchart_line_down\nchart_line_up\n\
chart_pie\nchart_pie_slice\nchart_polar\nchart_scatter\nchat\nchat_centered\n\
chat_centered_dots\nchat_centered_slash\nchat_centered_text\nchat_circle\n\
chat_circle_dots\nchat_circle_slash\nchat_circle_text\nchat_dots\nchat_slash\n\
chat_teardrop\nchat_teardrop_dots\nchat_teardrop_slash\nchat_teardrop_text\nchat_text\n\
chats\nchats_circle\nchats_teardrop\ncheck\ncheck_circle\ncheck_fat\ncheck_square\n\
check_square_offset\ncheckerboard\nchecks\ncheers\ncheese\nchef_hat\ncherries\nchurch\n\
cigarette\ncigarette_slash\ncircle\ncircle_dashed\ncircle_half\ncircle_half_tilt\n\
circle_notch\ncircle_wavy\ncircle_wavy_check\ncircle_wavy_question\ncircle_wavy_warning\n\
circles_four\ncircles_three\ncircles_three_plus\ncircuitry\ncity\nclipboard\n\
clipboard_text\nclock\nclock_afternoon\nclock_clockwise\nclock_countdown\n\
clock_counter_clockwise\nclock_user\nclosed_captioning\ncloud\ncloud_arrow_down\n\
cloud_arrow_up\ncloud_check\ncloud_fog\ncloud_lightning\ncloud_moon\ncloud_rain\n\
cloud_slash\ncloud_snow\ncloud_sun\ncloud_warning\ncloud_x\nclover\nclub\ncoat_hanger\n\
coda_logo\ncode\ncode_block\ncode_simple\ncodepen_logo\ncodesandbox_logo\ncoffee\n\
coffee_bean\ncoin\ncoin_vertical\ncoins\ncolumns\ncolumns_plus_left\ncolumns_plus_right\n\
command\ncompass\ncompass_rose\ncompass_tool\ncomputer_tower\nconfetti\n\
contactless_payment\ncontrol\ncookie\ncooking_pot\ncopy\ncopy_simple\ncopyleft\n\
copyright\ncorners_in\ncorners_out\ncouch\ncourt_basketball\ncow\ncowboy_hat\ncpu\ncrane\n\
crane_tower\ncredit_card\ncricket\ncrop\ncross\ncrosshair\ncrosshair_simple\ncrown\n\
crown_cross\ncrown_simple\ncube\ncube_focus\ncube_transparent\ncurrency_btc\n\
currency_circle_dollar\ncurrency_cny\ncurrency_dollar\ncurrency_dollar_simple\n\
currency_eth\ncurrency_eur\ncurrency_gbp\ncurrency_inr\ncurrency_jpy\ncurrency_krw\n\
currency_kzt\ncurrency_ngn\ncurrency_rub\ncursor\ncursor_click\ncursor_text\ncylinder\n\
database\ndesk\ndesktop\ndesktop_tower\ndetective\ndev_to_logo\ndevice_mobile\n\
device_mobile_camera\ndevice_mobile_slash\ndevice_mobile_speaker\ndevice_rotate\n\
device_tablet\ndevice_tablet_camera\ndevice_tablet_speaker\ndevices\ndiamond\n\
diamonds_four\ndice_five\ndice_four\ndice_one\ndice_six\ndice_three\ndice_two\ndisc\n\
disco_ball\ndiscord_logo\ndivide\ndna\ndog\ndoor\ndoor_open\ndot\ndot_outline\ndots_nine\n\
dots_six\ndots_six_vertical\ndots_three\ndots_three_circle\ndots_three_circle_vertical\n\
dots_three_outline\ndots_three_outline_vertical\ndots_three_vertical\ndownload\n\
download_simple\ndress\ndresser\ndribbble_logo\ndrone\ndrop\ndrop_half\ndrop_half_bottom\n\
drop_simple\ndrop_slash\ndropbox_logo\near\near_slash\negg\negg_crack\neject\n\
eject_simple\nelevator\nempty\nengine\nenvelope\nenvelope_open\nenvelope_simple\n\
envelope_simple_open\nequalizer\nequals\neraser\nescalator_down\nescalator_up\nexam\n\
exclamation_mark\nexclude\nexclude_square\nexport\neye\neye_closed\neye_slash\n\
eyedropper\neyedropper_sample\neyeglasses\neyes\nface_mask\nfacebook_logo\nfactory\n\
faders\nfaders_horizontal\nfallout_shelter\nfan\nfarm\nfast_forward\nfast_forward_circle\n\
feather\nfediverse_logo\nfigma_logo\nfile\nfile_archive\nfile_arrow_down\nfile_arrow_up\n\
file_audio\nfile_c\nfile_c_sharp\nfile_cloud\nfile_code\nfile_cpp\nfile_css\nfile_csv\n\
file_dashed\nfile_doc\nfile_dotted\nfile_html\nfile_image\nfile_ini\nfile_jpg\nfile_js\n\
file_jsx\nfile_lock\nfile_magnifying_glass\nfile_md\nfile_minus\nfile_pdf\nfile_plus\n\
file_png\nfile_ppt\nfile_py\nfile_rs\nfile_search\nfile_sql\nfile_svg\nfile_text\n\
file_ts\nfile_tsx\nfile_txt\nfile_video\nfile_vue\nfile_x\nfile_xls\nfile_zip\nfiles\n\
film_reel\nfilm_script\nfilm_slate\nfilm_strip\nfingerprint\nfingerprint_simple\n\
finn_the_human\nfire\nfire_extinguisher\nfire_simple\nfire_truck\nfirst_aid\n\
first_aid_kit\nfish\nfish_simple\nflag\nflag_banner\nflag_banner_fold\nflag_checkered\n\
flag_pennant\nflame\nflashlight\nflask\nflip_horizontal\nflip_vertical\nfloppy_disk\n\
floppy_disk_back\nflow_arrow\nflower\nflower_lotus\nflower_tulip\nflying_saucer\nfolder\n\
folder_dashed\nfolder_dotted\nfolder_lock\nfolder_minus\nfolder_notch\n\
folder_notch_minus\nfolder_notch_open\nfolder_notch_plus\nfolder_open\nfolder_plus\n\
folder_simple\nfolder_simple_dashed\nfolder_simple_dotted\nfolder_simple_lock\n\
folder_simple_minus\nfolder_simple_plus\nfolder_simple_star\nfolder_simple_user\n\
folder_star\nfolder_user\nfolders\nfootball\nfootball_helmet\nfootprints\nfork_knife\n\
four_k\nframe_corners\nframer_logo\nfunction\nfunnel\nfunnel_simple\nfunnel_simple_x\n\
funnel_x\ngame_controller\ngarage\ngas_can\ngas_pump\ngauge\ngavel\ngear\ngear_fine\n\
gear_six\ngender_female\ngender_intersex\ngender_male\ngender_neuter\ngender_nonbinary\n\
gender_transgender\nghost\ngif\ngift\ngit_branch\ngit_commit\ngit_diff\ngit_fork\n\
git_merge\ngit_pull_request\ngithub_logo\ngitlab_logo\ngitlab_logo_simple\nglobe\n\
globe_hemisphere_east\nglobe_hemisphere_west\nglobe_simple\nglobe_simple_x\nglobe_stand\n\
globe_x\ngoggles\ngolf\ngoodreads_logo\ngoogle_cardboard_logo\ngoogle_chrome_logo\n\
google_drive_logo\ngoogle_logo\ngoogle_photos_logo\ngoogle_play_logo\n\
google_podcasts_logo\ngps\ngps_fix\ngps_slash\ngradient\ngraduation_cap\ngrains\n\
grains_slash\ngraph\ngraphics_card\ngreater_than\ngreater_than_or_equal\ngrid_four\n\
grid_nine\nguitar\nhair_dryer\nhamburger\nhammer\nhand\nhand_arrow_down\nhand_arrow_up\n\
hand_coins\nhand_deposit\nhand_eye\nhand_fist\nhand_grabbing\nhand_heart\nhand_palm\n\
hand_peace\nhand_pointing\nhand_soap\nhand_swipe_left\nhand_swipe_right\nhand_tap\n\
hand_waving\nhand_withdraw\nhandbag\nhandbag_simple\nhands_clapping\nhands_praying\n\
handshake\nhard_drive\nhard_drives\nhard_hat\nhash\nhash_straight\nhead_circuit\n\
headlights\nheadphones\nheadset\nheart\nheart_break\nheart_half\nheart_straight\n\
heart_straight_break\nheartbeat\nhexagon\nhigh_definition\nhigh_heel\nhighlighter\n\
highlighter_circle\nhockey\nhoodie\nhorse\nhospital\nhourglass\nhourglass_high\n\
hourglass_low\nhourglass_medium\nhourglass_simple\nhourglass_simple_high\n\
hourglass_simple_low\nhourglass_simple_medium\nhouse\nhouse_line\nhouse_simple\n\
hurricane\nice_cream\nidentification_badge\nidentification_card\nimage\nimage_broken\n\
image_square\nimages\nimages_square\ninfinity\ninfo\ninstagram_logo\nintersect\n\
intersect_square\nintersect_three\nintersection\ninvoice\nisland\njar\njar_label\njeep\n\
joystick\nkanban\nkey\nkey_return\nkeyboard\nkeyhole\nknife\nladder\nladder_simple\nlamp\n\
lamp_pendant\nlaptop\nlasso\nlastfm_logo\nlayout\nleaf\nlectern\nlego\nlego_smiley\n\
lemniscate\nless_than\nless_than_or_equal\nletter_circle_h\nletter_circle_p\n\
letter_circle_v\nlifebuoy\nlightbulb\nlightbulb_filament\nlighthouse\nlightning\n\
lightning_a\nlightning_slash\nline_segment\nline_segments\nline_vertical\nlink\n\
link_break\nlink_simple\nlink_simple_break\nlink_simple_horizontal\n\
link_simple_horizontal_break\nlinkedin_logo\nlinktree_logo\nlinux_logo\nlist\n\
list_bullets\nlist_checks\nlist_dashes\nlist_heart\nlist_magnifying_glass\nlist_numbers\n\
list_plus\nlist_star\nlock\nlock_key\nlock_key_open\nlock_laminated\nlock_laminated_open\n\
lock_open\nlock_simple\nlock_simple_open\nlockers\nlog\nmagic_wand\nmagnet\n\
magnet_straight\nmagnifying_glass\nmagnifying_glass_minus\nmagnifying_glass_plus\n\
mailbox\nmap_pin\nmap_pin_area\nmap_pin_line\nmap_pin_plus\nmap_pin_simple\n\
map_pin_simple_area\nmap_pin_simple_line\nmap_trifold\nmarkdown_logo\nmarker_circle\n\
martini\nmask_happy\nmask_sad\nmastodon_logo\nmath_operations\nmatrix_logo\nmedal\n\
medal_military\nmedium_logo\nmegaphone\nmegaphone_simple\nmember_of\nmemory\n\
messenger_logo\nmeta_logo\nmeteor\nmetronome\nmicrophone\nmicrophone_slash\n\
microphone_stage\nmicroscope\nmicrosoft_excel_logo\nmicrosoft_outlook_logo\n\
microsoft_powerpoint_logo\nmicrosoft_teams_logo\nmicrosoft_word_logo\nminus\n\
minus_circle\nminus_square\nmoney\nmoney_wavy\nmonitor\nmonitor_arrow_up\nmonitor_play\n\
moon\nmoon_stars\nmoped\nmoped_front\nmosque\nmotorcycle\nmountains\nmouse\n\
mouse_left_click\nmouse_middle_click\nmouse_right_click\nmouse_scroll\nmouse_simple\n\
music_note\nmusic_note_simple\nmusic_notes\nmusic_notes_minus\nmusic_notes_plus\n\
music_notes_simple\nnavigation_arrow\nneedle\nnetwork\nnetwork_slash\nnetwork_x\n\
newspaper\nnewspaper_clipping\nnot_equals\nnot_member_of\nnot_subset_of\nnot_superset_of\n\
notches\nnote\nnote_blank\nnote_pencil\nnotebook\nnotepad\nnotification\nnotion_logo\n\
nuclear_plant\nnumber_circle_eight\nnumber_circle_five\nnumber_circle_four\n\
number_circle_nine\nnumber_circle_one\nnumber_circle_seven\nnumber_circle_six\n\
number_circle_three\nnumber_circle_two\nnumber_circle_zero\nnumber_eight\nnumber_five\n\
number_four\nnumber_nine\nnumber_one\nnumber_seven\nnumber_six\nnumber_square_eight\n\
number_square_five\nnumber_square_four\nnumber_square_nine\nnumber_square_one\n\
number_square_seven\nnumber_square_six\nnumber_square_three\nnumber_square_two\n\
number_square_zero\nnumber_three\nnumber_two\nnumber_zero\nnumpad\nnut\nny_times_logo\n\
octagon\noffice_chair\nonigiri\nopen_ai_logo\noption\norange\norange_slice\noven\n\
package\npaint_brush\npaint_brush_broad\npaint_brush_household\npaint_bucket\n\
paint_roller\npalette\npanorama\npants\npaper_plane\npaper_plane_right\npaper_plane_tilt\n\
paperclip\npaperclip_horizontal\nparachute\nparagraph\nparallelogram\npark\npassword\n\
path\npatreon_logo\npause\npause_circle\npaw_print\npaypal_logo\npeace\npen\npen_nib\n\
pen_nib_straight\npencil\npencil_circle\npencil_line\npencil_ruler\npencil_simple\n\
pencil_simple_line\npencil_simple_slash\npencil_slash\npentagon\npentagram\npepper\n\
percent\nperson\nperson_arms_spread\nperson_simple\nperson_simple_bike\n\
person_simple_circle\nperson_simple_hike\nperson_simple_run\nperson_simple_ski\n\
person_simple_snowboard\nperson_simple_swim\nperson_simple_tai_chi\nperson_simple_throw\n\
person_simple_walk\nperspective\nphone\nphone_call\nphone_disconnect\nphone_incoming\n\
phone_list\nphone_outgoing\nphone_pause\nphone_plus\nphone_slash\nphone_transfer\n\
phone_x\nphosphor_logo\npi\npiano_keys\npicnic_table\npicture_in_picture\npiggy_bank\n\
pill\nping_pong\npint_glass\npinterest_logo\npinwheel\npipe\npipe_wrench\npix_logo\n\
pizza\nplaceholder\nplanet\nplant\nplay\nplay_circle\nplay_pause\nplaylist\nplug\n\
plug_charging\nplugs\nplugs_connected\nplus\nplus_circle\nplus_minus\nplus_square\n\
poker_chip\npolice_car\npolygon\npopcorn\npopsicle\npotted_plant\npower\nprescription\n\
presentation\npresentation_chart\nprinter\nprohibit\nprohibit_inset\nprojector_screen\n\
projector_screen_chart\npulse\npush_pin\npush_pin_simple\npush_pin_simple_slash\n\
push_pin_slash\npuzzle_piece\nqr_code\nquestion\nquestion_mark\nqueue\nquotes\nrabbit\n\
racquet\nradical\nradio\nradio_button\nradioactive\nrainbow\nrainbow_cloud\nranking\n\
read_cv_logo\nreceipt\nreceipt_x\nrecord\nrectangle\nrectangle_dashed\nrecycle\n\
reddit_logo\nrepeat\nrepeat_once\nreplit_logo\nresize\nrewind\nrewind_circle\n\
road_horizon\nrobot\nrocket\nrocket_launch\nrows\nrows_plus_bottom\nrows_plus_top\nrss\n\
rss_simple\nrug\nruler\nsailboat\nscales\nscan\nscan_smiley\nscissors\nscooter\n\
screencast\nscrewdriver\nscribble\nscribble_loop\nscroll\nseal\nseal_check\nseal_percent\n\
seal_question\nseal_warning\nseat\nseatbelt\nsecurity_camera\nselection\nselection_all\n\
selection_background\nselection_foreground\nselection_inverse\nselection_plus\n\
selection_slash\nshapes\nshare\nshare_fat\nshare_network\nshield\nshield_check\n\
shield_checkered\nshield_chevron\nshield_plus\nshield_slash\nshield_star\nshield_warning\n\
shipping_container\nshirt_folded\nshooting_star\nshopping_bag\nshopping_bag_open\n\
shopping_cart\nshopping_cart_simple\nshovel\nshower\nshrimp\nshuffle\nshuffle_angular\n\
shuffle_simple\nsidebar\nsidebar_simple\nsigma\nsign_in\nsign_out\nsignature\nsignpost\n\
sim_card\nsiren\nsketch_logo\nskip_back\nskip_back_circle\nskip_forward\n\
skip_forward_circle\nskull\nskype_logo\nslack_logo\nsliders\nsliders_horizontal\n\
slideshow\nsmiley\nsmiley_angry\nsmiley_blank\nsmiley_meh\nsmiley_melting\n\
smiley_nervous\nsmiley_sad\nsmiley_sticker\nsmiley_wink\nsmiley_x_eyes\nsnapchat_logo\n\
sneaker\nsneaker_move\nsnowflake\nsoccer_ball\nsock\nsolar_panel\nsolar_roof\n\
sort_ascending\nsort_descending\nsoundcloud_logo\nspade\nsparkle\nspeaker_hifi\n\
speaker_high\nspeaker_low\nspeaker_none\nspeaker_simple_high\nspeaker_simple_low\n\
speaker_simple_none\nspeaker_simple_slash\nspeaker_simple_x\nspeaker_slash\nspeaker_x\n\
speedometer\nsphere\nspinner\nspinner_ball\nspinner_gap\nspiral\nsplit_horizontal\n\
split_vertical\nspotify_logo\nspray_bottle\nsquare\nsquare_half\nsquare_half_bottom\n\
square_logo\nsquare_split_horizontal\nsquare_split_vertical\nsquares_four\nstack\n\
stack_minus\nstack_overflow_logo\nstack_plus\nstack_simple\nstairs\nstamp\n\
standard_definition\nstar\nstar_and_crescent\nstar_four\nstar_half\nstar_of_david\n\
steam_logo\nsteering_wheel\nsteps\nstethoscope\nsticker\nstool\nstop\nstop_circle\n\
storefront\nstrategy\nstripe_logo\nstudent\nsubset_of\nsubset_proper_of\nsubtitles\n\
subtitles_slash\nsubtract\nsubtract_square\nsubway\nsuitcase\nsuitcase_rolling\n\
suitcase_simple\nsun\nsun_dim\nsun_horizon\nsunglasses\nsuperset_of\nsuperset_proper_of\n\
swap\nswatches\nswimming_pool\nsword\nsynagogue\nsyringe\nt_shirt\ntable\ntabs\ntag\n\
tag_chevron\ntag_simple\ntarget\ntaxi\ntea_bag\ntelegram_logo\ntelevision\n\
television_simple\ntennis_ball\ntent\nterminal\nterminal_window\ntest_tube\n\
text_a_underline\ntext_aa\ntext_align_center\ntext_align_justify\ntext_align_left\n\
text_align_right\ntext_b\ntext_bolder\ntext_columns\ntext_h\ntext_h_five\ntext_h_four\n\
text_h_one\ntext_h_six\ntext_h_three\ntext_h_two\ntext_indent\ntext_italic\ntext_outdent\n\
text_strikethrough\ntext_subscript\ntext_superscript\ntext_t\ntext_t_slash\n\
text_underline\ntextbox\nthermometer\nthermometer_cold\nthermometer_hot\n\
thermometer_simple\nthreads_logo\nthree_d\nthumbs_down\nthumbs_up\nticket\ntidal_logo\n\
tiktok_logo\ntilde\ntimer\ntip_jar\ntipi\ntire\ntoggle_left\ntoggle_right\ntoilet\n\
toilet_paper\ntoolbox\ntooth\ntornado\ntote\ntote_simple\ntowel\ntractor\ntrademark\n\
trademark_registered\ntraffic_cone\ntraffic_sign\ntraffic_signal\ntrain\ntrain_regional\n\
train_simple\ntram\ntranslate\ntrash\ntrash_simple\ntray\ntray_arrow_down\ntray_arrow_up\n\
treasure_chest\ntree\ntree_evergreen\ntree_palm\ntree_structure\ntree_view\ntrend_down\n\
trend_up\ntriangle\ntriangle_dashed\ntrolley\ntrolley_suitcase\ntrophy\ntruck\n\
truck_trailer\ntumblr_logo\ntwitch_logo\ntwitter_logo\numbrella\numbrella_simple\nunion\n\
unite\nunite_square\nupload\nupload_simple\nusb\nuser\nuser_check\nuser_circle\n\
user_circle_check\nuser_circle_dashed\nuser_circle_gear\nuser_circle_minus\n\
user_circle_plus\nuser_focus\nuser_gear\nuser_list\nuser_minus\nuser_plus\n\
user_rectangle\nuser_sound\nuser_square\nuser_switch\nusers\nusers_four\nusers_three\n\
van\nvault\nvector_three\nvector_two\nvibrate\nvideo\nvideo_camera\nvideo_camera_slash\n\
video_conference\nvignette\nvinyl_record\nvirtual_reality\nvirus\nvisor\nvoicemail\n\
volleyball\nwall\nwallet\nwarehouse\nwarning\nwarning_circle\nwarning_diamond\n\
warning_octagon\nwashing_machine\nwatch\nwave_sawtooth\nwave_sine\nwave_square\n\
wave_triangle\nwaveform\nwaveform_slash\nwaves\nwebcam\nwebcam_slash\nwebhooks_logo\n\
wechat_logo\nwhatsapp_logo\nwheelchair\nwheelchair_motion\nwifi_high\nwifi_low\n\
wifi_medium\nwifi_none\nwifi_slash\nwifi_x\nwind\nwindmill\nwindows_logo\nwine\nwrench\n\
x\nx_circle\nx_logo\nx_square\nyarn\nyin_yang\nyoutube_logo";

/// The codepoint of each name in [`NAMES`], in the same order. All are in the Private Use Area,
/// so a `u16` holds them.
pub const CODES: &[u16] = &[
    0xEB9A, 0xE000, 0xE6F8, 0xEE4E, 0xECD8, 0xE002, 0xE4FE, 0xE502, 0xE504, 0xE500, 0xE5D6, 0xE004,
    0xE006, 0xE8A6, 0xE506, 0xEB0C, 0xE50A, 0xEB0E, 0xE50C, 0xEB10, 0xE50E, 0xEAEE, 0xE510, 0xEB12,
    0xE512, 0xEB14, 0xE96C, 0xE572, 0xE514, 0xE5D8, 0xE008, 0xE7BC, 0xEB80, 0xE00A, 0xE974, 0xE5DA,
    0xE516, 0xEB96, 0xEDAA, 0xE00C, 0xE00E, 0xE010, 0xE012, 0xE014, 0xE016, 0xE03A, 0xE03C, 0xE018,
    0xE01A, 0xE01C, 0xE01E, 0xE020, 0xE022, 0xE024, 0xE026, 0xE028, 0xE02A, 0xE02C, 0xE05A, 0xE02E,
    0xE030, 0xE032, 0xE034, 0xE036, 0xE038, 0xE03E, 0xE040, 0xE042, 0xE044, 0xE046, 0xE048, 0xE04A,
    0xE04C, 0xE04E, 0xE050, 0xE052, 0xE054, 0xE056, 0xE518, 0xE51A, 0xE51C, 0xE51E, 0xE520, 0xE522,
    0xE524, 0xE526, 0xE528, 0xE52A, 0xE52C, 0xE52E, 0xE058, 0xE05C, 0xE05E, 0xE060, 0xE062, 0xE064,
    0xE066, 0xE068, 0xE06A, 0xE06C, 0xE06E, 0xE070, 0xE072, 0xE5DC, 0xE074, 0xE5DE, 0xE076, 0xE078,
    0xE07A, 0xE07C, 0xE07E, 0xE080, 0xE082, 0xE084, 0xE086, 0xE088, 0xE08A, 0xE08C, 0xE08E, 0xE090,
    0xE092, 0xE094, 0xE096, 0xE098, 0xEB06, 0xE09A, 0xE09C, 0xE530, 0xE532, 0xE09E, 0xE0A0, 0xED3E,
    0xE0A2, 0xE0A4, 0xE534, 0xE536, 0xE0A6, 0xED3C, 0xEB04, 0xE0A8, 0xE5E0, 0xE5E2, 0xEE34, 0xE0AA,
    0xE832, 0xE0AC, 0xE5E4, 0xEE04, 0xE9FC, 0xE774, 0xE818, 0xE922, 0xE0AE, 0xE0B0, 0xE5E6, 0xE76C,
    0xE0B2, 0xE0B4, 0xE0B6, 0xE0B8, 0xEC72, 0xE948, 0xE71A, 0xEA28, 0xEE4A, 0xE964, 0xE724, 0xE81E,
    0xE0BA, 0xE0BC, 0xE0BE, 0xE0C0, 0xE0C2, 0xE0C4, 0xE0C6, 0xE808, 0xEC50, 0xE7C6, 0xE7C4, 0xE7C2,
    0xE7BE, 0xE7C0, 0xE0C8, 0xE0CA, 0xED24, 0xEA2A, 0xE0CC, 0xE7B0, 0xEB62, 0xE7F4, 0xE0CE, 0xE5E8,
    0xE0D0, 0xE5EA, 0xE0D2, 0xE5EC, 0xE0D4, 0xE5EE, 0xEA2C, 0xEB00, 0xE0D6, 0xEE60, 0xEA64, 0xE9E0,
    0xE72C, 0xEDA0, 0xE0DA, 0xE0DC, 0xE0DE, 0xE0E0, 0xE786, 0xEE0A, 0xE7F2, 0xE0E2, 0xE0E4, 0xE0E6,
    0xE8F2, 0xEDE0, 0xE0E8, 0xE0EA, 0xE0EC, 0xE5F0, 0xE758, 0xECCA, 0xE722, 0xE6CE, 0xEAA4, 0xE8E4,
    0xEA34, 0xE00E, 0xEE54, 0xEA36, 0xE862, 0xE860, 0xE864, 0xE85E, 0xE74E, 0xE6B4, 0xE81C, 0xEA68,
    0xE0EE, 0xE5F2, 0xE0F2, 0xEC54, 0xE0F4, 0xE0F6, 0xE5F4, 0xE5F6, 0xE5F8, 0xE100, 0xE0FE, 0xE0FF,
    0xE102, 0xEC6C, 0xE106, 0xEA6E, 0xE49C, 0xE918, 0xEE34, 0xE780, 0xE538, 0xE108, 0xE10A, 0xE712,
    0xE7B2, 0xE7B4, 0xE8B0, 0xEA14, 0xE714, 0xEA12, 0xE8B2, 0xE10C, 0xE7DE, 0xE10E, 0xEC58, 0xE7A4,
    0xE110, 0xE9D8, 0xE112, 0xEE30, 0xE8CC, 0xE114, 0xE5FA, 0xE0F8, 0xEE50, 0xE116, 0xE118, 0xE11A,
    0xE11C, 0xE11E, 0xE120, 0xE122, 0xE124, 0xE13E, 0xE126, 0xE128, 0xE12A, 0xE12C, 0xE136, 0xE138,
    0xE134, 0xE132, 0xE130, 0xE12E, 0xE13A, 0xE13C, 0xE140, 0xED38, 0xED80, 0xED2E, 0xE9D0, 0xE748,
    0xE142, 0xE144, 0xE146, 0xE148, 0xE14A, 0xE14C, 0xE14E, 0xEBAA, 0xE766, 0xE950, 0xE5FC, 0xE5FE,
    0xE600, 0xEACA, 0xE8D0, 0xE150, 0xE152, 0xEAA6, 0xE154, 0xE8B6, 0xE156, 0xE158, 0xE15A, 0xEAA8,
    0xEAAC, 0xE15C, 0xE160, 0xE164, 0xE162, 0xE166, 0xE168, 0xE16C, 0xE16A, 0xE16E, 0xE170, 0xE15E,
    0xE172, 0xE176, 0xE174, 0xE178, 0xE17A, 0xE17C, 0xE17E, 0xE180, 0xE182, 0xE184, 0xEBA6, 0xE186,
    0xE188, 0xE8C4, 0xE53A, 0xEA4A, 0xE9FE, 0xED8E, 0xE830, 0xECEA, 0xED90, 0xED92, 0xE18A, 0xE602,
    0xE18C, 0xE18E, 0xEB44, 0xE604, 0xE606, 0xE608, 0xE60C, 0xE190, 0xE192, 0xE194, 0xE9C2, 0xEA6A,
    0xE196, 0xE198, 0xE19A, 0xE19C, 0xE19E, 0xED2C, 0xE1A0, 0xEDEC, 0xE1A4, 0xE1AA, 0xE1AC, 0xE1AE,
    0xE1B0, 0xE53C, 0xE1B2, 0xE53E, 0xE1B4, 0xE1B6, 0xE1B8, 0xE540, 0xEA98, 0xEA96, 0xEDC8, 0xE1BA,
    0xE7FE, 0xE7CE, 0xE1BC, 0xEAFE, 0xE1BE, 0xE978, 0xEA06, 0xE1C2, 0xE1C0, 0xE60E, 0xEB48, 0xE78E,
    0xE546, 0xE544, 0xE542, 0xE1C4, 0xE1C8, 0xE1C6, 0xEA0E, 0xE548, 0xE81A, 0xED42, 0xECA6, 0xE6CA,
    0xE764, 0xE1CA, 0xE1CC, 0xE86A, 0xE54A, 0xE1CE, 0xE1D0, 0xE7F6, 0xEE36, 0xEABE, 0xED12, 0xE610,
    0xED48, 0xED49, 0xE1D2, 0xEE12, 0xE1D4, 0xE8A0, 0xE1D6, 0xE1D8, 0xE614, 0xEE5E, 0xE616, 0xE1DA,
    0xED0A, 0xEC7C, 0xE618, 0xE54C, 0xE54E, 0xE550, 0xE552, 0xEADA, 0xE554, 0xE556, 0xE558, 0xE55A,
    0xE55C, 0xEC4C, 0xEB52, 0xE55E, 0xE1DC, 0xE7C8, 0xE7D8, 0xE8FC, 0xE1DE, 0xED16, 0xE560, 0xE562,
    0xE83E, 0xED0E, 0xE1E0, 0xE1E2, 0xEE46, 0xE1E4, 0xEDF2, 0xE1E6, 0xE1E8, 0xE1EA, 0xEBA4, 0xE1EC,
    0xE8F4, 0xE1EE, 0xE1F0, 0xE1F2, 0xE1F4, 0xE1F6, 0xE1F8, 0xE564, 0xED98, 0xE61A, 0xE1FA, 0xE924,
    0xE74A, 0xE61C, 0xE7E6, 0xECDE, 0xECE0, 0xE1FC, 0xE794, 0xEAE2, 0xE1FE, 0xE200, 0xE202, 0xE204,
    0xE206, 0xE208, 0xE20A, 0xE20C, 0xEA7E, 0xE94E, 0xE20E, 0xED74, 0xE210, 0xE566, 0xEB40, 0xEE32,
    0xE954, 0xE7D0, 0xE70C, 0xE70E, 0xE812, 0xEB64, 0xE212, 0xE6AE, 0xECC0, 0xEDBC, 0xEA80, 0xE214,
    0xE216, 0xE218, 0xE21A, 0xEBBC, 0xE21C, 0xE21E, 0xECBA, 0xECBC, 0xE742, 0xEE44, 0xE882, 0xE880,
    0xEAF0, 0xE220, 0xE222, 0xE224, 0xE568, 0xEAC4, 0xE7BA, 0xEE5C, 0xE56A, 0xE226, 0xE760, 0xE228,
    0xE22A, 0xE9DE, 0xE9F2, 0xEC70, 0xE6A6, 0xE22C, 0xE9C0, 0xED66, 0xE22E, 0xE230, 0xEB2A, 0xE232,
    0xE61E, 0xEA20, 0xEB32, 0xEB30, 0xE95E, 0xE914, 0xEB2E, 0xEB34, 0xEB1C, 0xE704, 0xEB1E, 0xE704,
    0xEB38, 0xEA24, 0xEB33, 0xEB1A, 0xEB24, 0xEB3A, 0xE95C, 0xE238, 0xED50, 0xE234, 0xE702, 0xE236,
    0xEB18, 0xEB20, 0xEB2C, 0xEB28, 0xE238, 0xED4E, 0xED08, 0xE23A, 0xEB26, 0xEB3C, 0xEB35, 0xEA22,
    0xEB3E, 0xE23C, 0xEB22, 0xE958, 0xE710, 0xE8C0, 0xEB50, 0xE8C2, 0xE792, 0xE23E, 0xE240, 0xE56C,
    0xE242, 0xE9E8, 0xE620, 0xE574, 0xE56E, 0xE570, 0xE728, 0xE72A, 0xE244, 0xE622, 0xECF2, 0xEA38,
    0xECF0, 0xE624, 0xE246, 0xE79E, 0xED6A, 0xED6C, 0xE248, 0xEAF4, 0xE6EC, 0xE75E, 0xE6CC, 0xEACC,
    0xEB4A, 0xE24A, 0xE8F8, 0xE8F8, 0xEA3C, 0xE254, 0xE24A, 0xE254, 0xE256, 0xE258, 0xE256, 0xE258,
    0xE25A, 0xEC2A, 0xEC2A, 0xEB5E, 0xE25C, 0xE25E, 0xEC2E, 0xEB60, 0xEA86, 0xEB46, 0xE260, 0xE718,
    0xEE4C, 0xEA88, 0xE262, 0xEA5C, 0xE626, 0xE264, 0xEBE4, 0xE266, 0xE268, 0xE26A, 0xE26C, 0xE26E,
    0xECD6, 0xE8CE, 0xE768, 0xE628, 0xEA32, 0xE270, 0xE87C, 0xE272, 0xE6E0, 0xE6E6, 0xE6E2, 0xE6EA,
    0xE6E4, 0xE6E8, 0xE62A, 0xE274, 0xE276, 0xE278, 0xE27A, 0xE27C, 0xE27E, 0xE280, 0xE282, 0xE576,
    0xE694, 0xE696, 0xE288, 0xE28A, 0xE28C, 0xE28E, 0xE284, 0xE290, 0xE286, 0xECB4, 0xEA3E, 0xED10,
    0xE7B6, 0xE976, 0xE8F6, 0xE292, 0xEB92, 0xE294, 0xEB94, 0xEDD8, 0xEDD6, 0xEDD4, 0xEB42, 0xE62C,
    0xEC68, 0xEC6A, 0xEB58, 0xE612, 0xEDC4, 0xEDA2, 0xE296, 0xEC8C, 0xEA8A, 0xEA66, 0xE790, 0xE80E,
    0xE298, 0xEA4E, 0xEE5A, 0xEA8C, 0xEE82, 0xEA4C, 0xE57A, 0xE57C, 0xE810, 0xE57E, 0xE7CC, 0xE29A,
    0xE630, 0xEC94, 0xEC92, 0xEC90, 0xE580, 0xEE80, 0xE29C, 0xE62E, 0xE6A0, 0xECC8, 0xE582, 0xE29E,
    0xE2A0, 0xED46, 0xE2A2, 0xE2A4, 0xE7D4, 0xE6FE, 0xE2A6, 0xE584, 0xE2A8, 0xEBE8, 0xEC48, 0xE2AA,
    0xEB98, 0xE2AC, 0xE2AE, 0xEA8E, 0xE8E8, 0xEC76, 0xE632, 0xEC86, 0xECD0, 0xE2B0, 0xE844, 0xE2B2,
    0xE2B4, 0xE2B6, 0xE2B8, 0xE2BA, 0xE2BC, 0xE2BE, 0xE2C0, 0xE2C2, 0xE2C4, 0xE2C6, 0xE88E, 0xE804,
    0xE6F6, 0xE2C8, 0xE2CA, 0xE7A8, 0xE2CC, 0xE836, 0xE834, 0xE634, 0xE2CE, 0xE2D0, 0xE2D2, 0xE87A,
    0xECC4, 0xEDBA, 0xEE42, 0xEE06, 0xE7E0, 0xE7E1, 0xE2D4, 0xEA5E, 0xEB54, 0xE2D6, 0xE782, 0xE2D8,
    0xEA78, 0xE636, 0xE9E4, 0xEC26, 0xE638, 0xEE2E, 0xE586, 0xEDC6, 0xE842, 0xE6D6, 0xE2DA, 0xE95A,
    0xE8C6, 0xE8C7, 0xE634, 0xEDAC, 0xEDA4, 0xEBF8, 0xEC08, 0xEC14, 0xE63A, 0xE2DC, 0xE63C, 0xE9F6,
    0xE2DE, 0xEA84, 0xE2E0, 0xE6D2, 0xE6D4, 0xED70, 0xE2E2, 0xE2E4, 0xE2E6, 0xE2E8, 0xE2EA, 0xE2EC,
    0xE2EE, 0xEDEE, 0xEB02, 0xE2F0, 0xE2F2, 0xEADC, 0xE2F4, 0xEBDE, 0xEBE0, 0xE2F6, 0xE2F8, 0xEBDC,
    0xE2FA, 0xE2FE, 0xE300, 0xE302, 0xE304, 0xE306, 0xE308, 0xE30A, 0xECB8, 0xED82, 0xE6B6, 0xE680,
    0xE682, 0xE30C, 0xE30E, 0xE310, 0xEC1E, 0xE316, 0xEE3A, 0xE318, 0xE314, 0xEE3E, 0xEE3C, 0xEE38,
    0xE31A, 0xE508, 0xE640, 0xE31C, 0xE9F4, 0xEB9E, 0xED68, 0xE31E, 0xED64, 0xE320, 0xECFC, 0xE322,
    0xE324, 0xE642, 0xEDC2, 0xE9C4, 0xE6D8, 0xED02, 0xE9BA, 0xEC8E, 0xE326, 0xE328, 0xE75C, 0xEC7A,
    0xEB6C, 0xEB70, 0xEACE, 0xEB66, 0xEB6A, 0xE32A, 0xE32C, 0xED4C, 0xE588, 0xEE68, 0xE32E, 0xE58A,
    0xE58C, 0xE330, 0xE58E, 0xE824, 0xE822, 0xECEE, 0xE80A, 0xE7AE, 0xE33A, 0xE334, 0xE338, 0xE336,
    0xE332, 0xE644, 0xE33C, 0xE33E, 0xE340, 0xEE0C, 0xEB7C, 0xE342, 0xEADE, 0xE82E, 0xEDDE, 0xEDDC,
    0xEDDA, 0xE344, 0xE346, 0xEDA6, 0xEDAE, 0xEDB0, 0xEDB2, 0xED3A, 0xE348, 0xE34A, 0xE34C, 0xE34E,
    0xE63E, 0xE6FA, 0xE9A0, 0xED7C, 0xE352, 0xE358, 0xE35E, 0xE364, 0xE36A, 0xE370, 0xE376, 0xE37C,
    0xE382, 0xE388, 0xE350, 0xE356, 0xE35C, 0xE362, 0xE368, 0xE36E, 0xE374, 0xE354, 0xE35A, 0xE360,
    0xE366, 0xE36C, 0xE372, 0xE378, 0xE37E, 0xE384, 0xE38A, 0xE37A, 0xE380, 0xE386, 0xE3C8, 0xE38C,
    0xE646, 0xE38E, 0xEA46, 0xEE2C, 0xE7D2, 0xE8A8, 0xEE40, 0xED36, 0xED8C, 0xE390, 0xE6F0, 0xE590,
    0xE6F2, 0xE392, 0xE6F4, 0xE6C8, 0xEAA2, 0xEC88, 0xE394, 0xE396, 0xE398, 0xE39A, 0xE592, 0xEA7C,
    0xE960, 0xECC6, 0xECB2, 0xE752, 0xE39C, 0xE98A, 0xE39E, 0xE3A0, 0xE648, 0xE98C, 0xE3A2, 0xE3AA,
    0xE3AC, 0xE64A, 0xE3AE, 0xE3B0, 0xE3B2, 0xE906, 0xE3B4, 0xEBC6, 0xECF6, 0xECF8, 0xEC7E, 0xEC5C,
    0xE94A, 0xE3B6, 0xE3A8, 0xECFE, 0xE72E, 0xE734, 0xEE58, 0xED54, 0xE730, 0xE71C, 0xE71E, 0xE736,
    0xED5C, 0xE732, 0xE73A, 0xEBE6, 0xE3B8, 0xE3BA, 0xE3BC, 0xE3BE, 0xE3CC, 0xE3C0, 0xE3CA, 0xEC56,
    0xE3C2, 0xE3C6, 0xE3C4, 0xE3CE, 0xEC80, 0xE9C8, 0xEE26, 0xE64C, 0xEA04, 0xE700, 0xEA42, 0xEDD0,
    0xE64E, 0xEB9C, 0xED86, 0xED88, 0xECC2, 0xE796, 0xE650, 0xE652, 0xEBAE, 0xE3D0, 0xE3D2, 0xE8BE,
    0xE6AA, 0xE946, 0xEB5C, 0xEB56, 0xEB5A, 0xE3D4, 0xE3D6, 0xE3D8, 0xED4A, 0xE594, 0xEC4A, 0xE6D0,
    0xEB4E, 0xEBBE, 0xEC22, 0xE3DA, 0xE7A2, 0xE654, 0xE656, 0xE3DC, 0xE3DE, 0xE3E0, 0xE658, 0xE65A,
    0xE000, 0xE3E2, 0xE65C, 0xE65E, 0xE3E4, 0xE596, 0xE3E6, 0xE3E8, 0xE3E9, 0xE6AC, 0xE660, 0xEAC2,
    0xEE02, 0xE3EA, 0xE77E, 0xEB08, 0xE9DC, 0xE598, 0xE59A, 0xED62, 0xED0C, 0xE3EC, 0xED40, 0xE3EE,
    0xE3F0, 0xE3F2, 0xE75A, 0xE59C, 0xE3F6, 0xE3F8, 0xEB8A, 0xED6E, 0xE6A8, 0xE3FA, 0xE838, 0xE762,
    0xE3FC, 0xE3FE, 0xE5A2, 0xE59E, 0xE5A0, 0xE400, 0xE402, 0xEA1A, 0xE6B8, 0xE78A, 0xE750, 0xEBB6,
    0xEBB4, 0xEAE0, 0xE820, 0xE404, 0xE86E, 0xE806, 0xE662, 0xEB7A, 0xE604, 0xE606, 0xE60A, 0xE608,
    0xE60C, 0xEB8E, 0xEDFE, 0xECA4, 0xE69A, 0xE746, 0xEAF8, 0xEAF6, 0xE744, 0xE69C, 0xE69E, 0xEC5E,
    0xE406, 0xED52, 0xE408, 0xE40A, 0xE40C, 0xE708, 0xE40E, 0xE706, 0xE410, 0xEC34, 0xE412, 0xE78C,
    0xEA92, 0xECFA, 0xE416, 0xE418, 0xE41E, 0xE420, 0xE9E6, 0xE776, 0xEAB4, 0xE422, 0xE424, 0xE426,
    0xEAB6, 0xEC24, 0xEAB8, 0xE428, 0xE42A, 0xEBAC, 0xE89C, 0xE664, 0xE9B8, 0xE42C, 0xE5A4, 0xE42E,
    0xE5A6, 0xE430, 0xE916, 0xE8DC, 0xE5A8, 0xE432, 0xE434, 0xED32, 0xE436, 0xEC62, 0xE438, 0xE43A,
    0xEE56, 0xE43C, 0xE43E, 0xE440, 0xE666, 0xE442, 0xE668, 0xE80C, 0xED60, 0xE5AA, 0xE716, 0xECCE,
    0xED7A, 0xED7B, 0xE444, 0xE446, 0xE8DE, 0xE448, 0xE6A2, 0xEA08, 0xE44A, 0xE44C, 0xE44E, 0xE450,
    0xE452, 0xE454, 0xE456, 0xE458, 0xE45A, 0xE45C, 0xEE74, 0xEE66, 0xE66A, 0xEE28, 0xE66C, 0xE9FA,
    0xE872, 0xE876, 0xE66E, 0xE7E4, 0xE45E, 0xE462, 0xEB16, 0xE690, 0xE870, 0xE874, 0xE464, 0xE466,
    0xEDF4, 0xEB78, 0xEDF6, 0xE468, 0xE8EC, 0xEA48, 0xEA90, 0xE46A, 0xECF4, 0xE6A4, 0xE70A, 0xE89E,
    0xEAD4, 0xE9AC, 0xECBE, 0xE7EA, 0xE5AC, 0xEA44, 0xE46C, 0xE46E, 0xE470, 0xEA3A, 0xE698, 0xE73E,
    0xEDC0, 0xEDB6, 0xE1A8, 0xE1A6, 0xEBD6, 0xEBD4, 0xE498, 0xE5AE, 0xE9B0, 0xE5B0, 0xE472, 0xE474,
    0xE5B6, 0xE816, 0xEDB8, 0xEDB4, 0xE83C, 0xE5B8, 0xECB6, 0xE5BA, 0xECEC, 0xE968, 0xE670, 0xE476,
    0xE778, 0xE478, 0xE672, 0xE47A, 0xE47C, 0xE902, 0xE8E6, 0xE5BC, 0xE754, 0xEAE6, 0xE720, 0xE8BA,
    0xE47E, 0xEAE8, 0xE7A0, 0xED34, 0xE6EE, 0xE480, 0xE482, 0xE484, 0xE486, 0xE5BE, 0xE5BE, 0xEC96,
    0xE6BA, 0xE6C4, 0xE6C2, 0xE6BC, 0xE6C6, 0xE6C0, 0xE6BE, 0xEA1E, 0xE5C0, 0xEA1C, 0xE5C2, 0xEC98,
    0xEC9A, 0xE48A, 0xE488, 0xE5C4, 0xEB0A, 0xE5C6, 0xE5C8, 0xE5CA, 0xE5CC, 0xED9E, 0xEA5A, 0xE48C,
    0xE48E, 0xE490, 0xED1C, 0xEAF2, 0xEDA8, 0xE492, 0xE7E2, 0xED30, 0xEDD2, 0xE674, 0xE676, 0xE79A,
    0xE79C, 0xECA0, 0xE9CC, 0xE88C, 0xE494, 0xE678, 0xEDE6, 0xEC6E, 0xE9F0, 0xE3F4, 0xE9A8, 0xE67A,
    0xE9AA, 0xE496, 0xE49E, 0xE4A0, 0xE9EC, 0xE4A2, 0xE4A6, 0xE4A8, 0xE4AA, 0xE010, 0xEE52, 0xEDE2,
    0xE6DA, 0xE6DC, 0xE91A, 0xE67C, 0xEE48, 0xE4AC, 0xE4AE, 0xE4B0, 0xE4B2, 0xE5B2, 0xE5B4, 0xE67E,
    0xE4B4, 0xE4B6, 0xE8D4, 0xE5CE, 0xE4BA, 0xE684, 0xE686, 0xEDBE, 0xE87E, 0xE878, 0xE4BE, 0xE4C0,
    0xE956, 0xE4C2, 0xEAFA, 0xE4C4, 0xEC38, 0xEC36, 0xE4C6, 0xE4C8, 0xE4CA, 0xE6FC, 0xE4CC, 0xE73C,
    0xE4CE, 0xE4D0, 0xE4D2, 0xECA8, 0xE4D4, 0xE756, 0xE4D6, 0xE68C, 0xE68E, 0xE826, 0xE76E, 0xEE62,
    0xEE64, 0xE4D8, 0xE740, 0xE4DA, 0xE4DC, 0xEDCE, 0xEBA2, 0xECAC, 0xE7B8, 0xE7D6, 0xEE2A, 0xE4DE,
    0xE726, 0xE688, 0xE68A, 0xECD4, 0xE4E0, 0xE4E2, 0xE7FC, 0xE4E4, 0xEDE8, 0xE4E6, 0xEA9C, 0xEA9A,
    0xEA9E, 0xEAA0, 0xE802, 0xE800, 0xE6DE, 0xE9B2, 0xECDC, 0xECAE, 0xE8D2, 0xE5D0, 0xE4E8, 0xE89A,
    0xE4EA, 0xE4EC, 0xE4EE, 0xE4F0, 0xE4F2, 0xE4F4, 0xE5D2, 0xE9F8, 0xE692, 0xE6B2, 0xE5D4, 0xE4F6,
    0xE4F8, 0xE4BC, 0xE4FA, 0xED9A, 0xE92A, 0xE4FC,
];

/// The glyph for a shortcode name. `-` and `_` are the same, so Phosphor's own
/// `folder-open` works as well as `folder_open`.
pub fn lookup(name: &str) -> Option<char> {
    if name.is_empty() || name.len() > 40 {
        return None;
    }
    let key: String = name
        .chars()
        .map(|c| {
            if c == '-' {
                '_'
            } else {
                c.to_ascii_lowercase()
            }
        })
        .collect();
    let index = NAMES.split('\n').position(|n| n == key)?;
    CODES.get(index).and_then(|&c| char::from_u32(u32::from(c)))
}
