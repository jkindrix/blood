void blood_init_args(int argc, char **argv);
int blood_main(void);
int main(int argc, char **argv) {
    blood_init_args(argc, argv);
    return blood_main();
}
